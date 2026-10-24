package com.telecomfort.ui.screens

import android.hardware.Camera
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
 * Живое превью камеры без CameraX/сторонних зависимостей:
 * android.hardware.Camera (API 1) + SurfaceView. Снимок → JPEG в cacheDir.
 * Видео остаётся системной камерой (MediaRecorder-поток не дублируем).
 */
@Composable
fun InAppCamera(onCapture: (File) -> Unit, onClose: () -> Unit) {
    val ctx = LocalContext.current
    var camera by remember { mutableStateOf<Camera?>(null) }
    var ready by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    val surfaceView = remember { SurfaceView(ctx) }

    DisposableEffect(Unit) {
        val holder: SurfaceHolder = surfaceView.holder
        val callback = object : SurfaceHolder.Callback {
            override fun surfaceCreated(h: SurfaceHolder) {
                runCatching {
                    val cam = Camera.open()
                    cam.setDisplayOrientation(90)
                    val params = cam.parameters
                    if (params.supportedFocusModes?.contains(Camera.Parameters.FOCUS_MODE_CONTINUOUS_PICTURE) == true) {
                        params.focusMode = Camera.Parameters.FOCUS_MODE_CONTINUOUS_PICTURE
                    }
                    cam.parameters = params
                    cam.setPreviewDisplay(h)
                    cam.startPreview()
                    camera = cam
                    ready = true
                }.onFailure {
                    error = "Камера недоступна: ${it.message}"
                }
            }

            override fun surfaceChanged(h: SurfaceHolder, format: Int, w: Int, hh: Int) {}
            override fun surfaceDestroyed(h: SurfaceHolder) {
                camera?.release()
                camera = null
                ready = false
            }
        }
        holder.addCallback(callback)
        onDispose {
            holder.removeCallback(callback)
            camera?.release()
            camera = null
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
                            val cam = camera
                            if (cam != null) {
                                cam.takePicture(null, null) { data, _ ->
                                    runCatching {
                                        val f = File(ctx.cacheDir, "cam_${System.currentTimeMillis()}.jpg")
                                        f.writeBytes(data)
                                        onCapture(f)
                                    }.onFailure { error = "Снимок не сохранён: ${it.message}" }
                                }
                            }
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
