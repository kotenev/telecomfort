package com.telecomfort.ui.screens

import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import java.io.File

/**
 * Держатель Camera2-ресурсов между recomposition; закрывается в onDispose.
 */
private class Camera2Refs {
    var thread: HandlerThread? = null
    var handler: Handler? = null
    var device: CameraDevice? = null
    var session: CameraCaptureSession? = null
    var reader: ImageReader? = null
    var jpegOrientation: Int = 0
}

/*
 * Старая перегрузка createCaptureSession(List, StateCallback, Handler) помечена
 * устаревшей в пользу SessionConfiguration (API 28+). Оставляем её намеренно ради
 * minSdk 26 без разветвления runtime-классов, поэтому предупреждение подавлено точечно.
 */
@Suppress("DEPRECATION")
private fun createPreviewSession(
    device: CameraDevice,
    surfaces: List<Surface>,
    callback: CameraCaptureSession.StateCallback,
    handler: Handler?,
) {
    device.createCaptureSession(surfaces, callback, handler)
}

/**
 * Живое превью камеры на Camera2 (без сторонних зависимостей):
 * задняя камера + SurfaceView. Снимок JPEG → cacheDir.
 * Видео остаётся системной камерой (MediaRecorder-поток не дублируем).
 */
@Composable
fun InAppCamera(onCapture: (File) -> Unit, onClose: () -> Unit) {
    val ctx = LocalContext.current
    val mainHandler = remember { Handler(Looper.getMainLooper()) }
    val refs = remember { Camera2Refs() }
    var ready by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    val surfaceView = remember { SurfaceView(ctx) }

    DisposableEffect(Unit) {
        val holder: SurfaceHolder = surfaceView.holder

        fun postError(message: String) {
            mainHandler.post { error = message }
        }

        fun closeCamera() {
            runCatching { refs.session?.close() }
            refs.session = null
            runCatching { refs.device?.close() }
            refs.device = null
            runCatching { refs.reader?.close() }
            refs.reader = null
            refs.thread?.quitSafely()
            runCatching { refs.thread?.join(1000) }
            refs.thread = null
            refs.handler = null
        }

        fun openCamera(previewHolder: SurfaceHolder) {
            closeCamera()
            val manager = ctx.getSystemService(Context.CAMERA_SERVICE) as CameraManager
            val thread = HandlerThread("TelecomfortCamera").apply { start() }
            val handler = Handler(thread.looper)
            refs.thread = thread
            refs.handler = handler

            val cameraId = manager.cameraIdList.firstOrNull { id ->
                manager.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_FACING) ==
                    CameraCharacteristics.LENS_FACING_BACK
            } ?: manager.cameraIdList.firstOrNull() ?: run {
                postError("Камера недоступна: устройство без камеры")
                return
            }
            val characteristics = manager.getCameraCharacteristics(cameraId)
            val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            val jpegSize = map?.getOutputSizes(ImageFormat.JPEG)
                ?.maxByOrNull { it.width * it.height } ?: run {
                postError("Камера недоступна: нет JPEG-размера")
                return
            }
            refs.jpegOrientation =
                characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0
            val reader = ImageReader.newInstance(jpegSize.width, jpegSize.height, ImageFormat.JPEG, 2)
            refs.reader = reader
            reader.setOnImageAvailableListener({ r ->
                val image = r.acquireLatestImage() ?: return@setOnImageAvailableListener
                try {
                    val buffer = image.planes[0].buffer
                    val bytes = ByteArray(buffer.remaining())
                    buffer.get(bytes)
                    val file = File(ctx.cacheDir, "cam_${System.currentTimeMillis()}.jpg")
                    file.writeBytes(bytes)
                    mainHandler.post { onCapture(file) }
                } catch (e: Exception) {
                    postError("Снимок не сохранён: ${e.message}")
                } finally {
                    image.close()
                }
            }, handler)

            manager.openCamera(cameraId, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    refs.device = camera
                    val surfaces = listOf(previewHolder.surface, reader.surface)
                    createPreviewSession(camera, surfaces, object : CameraCaptureSession.StateCallback() {
                        override fun onConfigured(session: CameraCaptureSession) {
                            refs.session = session
                            try {
                                val preview = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                                    addTarget(previewHolder.surface)
                                    val afModes = characteristics.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES)
                                    if (afModes != null && afModes.contains(CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)) {
                                        set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                                    }
                                }.build()
                                session.setRepeatingRequest(preview, null, handler)
                                mainHandler.post { ready = true }
                            } catch (e: Exception) {
                                postError("Камера недоступна: ${e.message}")
                            }
                        }

                        override fun onConfigureFailed(session: CameraCaptureSession) {
                            postError("Камера недоступна: не удалось настроить превью")
                        }
                    }, handler)
                }

                override fun onDisconnected(camera: CameraDevice) {
                    runCatching { camera.close() }
                }

                override fun onError(camera: CameraDevice, errorCode: Int) {
                    runCatching { camera.close() }
                    postError("Камера недоступна: ошибка $errorCode")
                }
            }, handler)
        }

        val callback = object : SurfaceHolder.Callback {
            override fun surfaceCreated(h: SurfaceHolder) {
                runCatching { openCamera(h) }.onFailure {
                    postError("Камера недоступна: ${it.message}")
                }
            }

            override fun surfaceChanged(h: SurfaceHolder, format: Int, w: Int, hh: Int) {}

            override fun surfaceDestroyed(h: SurfaceHolder) {
                closeCamera()
                mainHandler.post { ready = false }
            }
        }
        holder.addCallback(callback)
        onDispose {
            holder.removeCallback(callback)
            closeCamera()
            ready = false
        }
    }

    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(color = Color.Black, modifier = Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize()) {
                AndroidView(factory = { surfaceView }, modifier = Modifier.fillMaxSize())
                Row(
                    Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(16.dp),
                    horizontalArrangement = Arrangement.SpaceEvenly,
                ) {
                    TextButton(onClick = onClose) { Text("✕", color = Color.White) }
                    Button(
                        enabled = ready,
                        onClick = {
                            val device = refs.device
                            val session = refs.session
                            val reader = refs.reader
                            val handler = refs.handler
                            if (device == null || session == null || reader == null || handler == null) {
                                error = "Камера ещё не готова"
                                return@Button
                            }
                            runCatching {
                                val capture = device.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE).apply {
                                    addTarget(reader.surface)
                                    set(CaptureRequest.JPEG_ORIENTATION, refs.jpegOrientation)
                                }.build()
                                session.capture(capture, null, handler)
                            }.onFailure { error = "Снимок не сохранён: ${it.message}" }
                        },
                    ) { Text("📸 Снять") }
                    Text("", Modifier.width(48.dp))
                }
                if (error.isNotEmpty()) {
                    Text(error, color = Color.White, modifier = Modifier.align(Alignment.Center).padding(16.dp))
                }
            }
        }
    }
}
