package com.telecomfort.core

import android.Manifest
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.core.content.ContextCompat
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/**
 * Запись PCM 16-bit моно 48кГц через AudioRecord + свой WAV-райтер.
 * MediaRecorder не умеет WAV — его 3gp/AMR rodio не читает.
 */
class PcmRecorder private constructor(
    private val rec: AudioRecord,
    private val file: File,
    private val running: AtomicBoolean,
    private val worker: Thread,
) {
    var level: Float = 0f
        private set

    fun stop(): File {
        running.set(false)
        worker.join(3000)
        try { rec.stop() } catch (_: Exception) {}
        try { rec.release() } catch (_: Exception) {}
        fixWavHeader(file)
        return file
    }

    companion object {
        const val RATE = 48000

        fun start(dst: File): PcmRecorder {
            val minBuf = AudioRecord.getMinBufferSize(
                RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
            )
            require(minBuf > 0) { "нет буфера AudioRecord ($minBuf)" }
            val rec = AudioRecord(
                MediaRecorder.AudioSource.MIC, RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
                minBuf * 4,
            )
            require(rec.state == AudioRecord.STATE_INITIALIZED) { "AudioRecord не инициализирован" }
            dst.parentFile?.mkdirs()
            val out = FileOutputStream(dst)
            writeWavHeader(out, 0)
            val running = AtomicBoolean(true)
            var me: PcmRecorder? = null
            val worker = thread(name = "pcm-rec", isDaemon = true) {
                val buf = ShortArray(960 * 4)
                var total = 0
                rec.startRecording()
                while (running.get()) {
                    val n = rec.read(buf, 0, buf.size)
                    if (n > 0) {
                        val bytes = ByteArray(n * 2)
                        var peak = 0
                        for (i in 0 until n) {
                            val v = buf[i].toInt()
                            if (kotlin.math.abs(v) > peak) peak = kotlin.math.abs(v)
                            bytes[i * 2] = (v and 0xFF).toByte()
                            bytes[i * 2 + 1] = ((v shr 8) and 0xFF).toByte()
                        }
                        out.write(bytes)
                        total += n
                        me?.level = (peak / 32768f * 3f).coerceIn(0f, 1f)
                    }
                }
                out.flush()
                out.close()
            }
            val r = PcmRecorder(rec, dst, running, worker)
            me = r
            return r
        }

        private fun writeWavHeader(out: FileOutputStream, dataBytes: Int) {
            val h = java.nio.ByteBuffer.allocate(44).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            h.put("RIFF".toByteArray())
            h.putInt(36 + dataBytes)
            h.put("WAVEfmt ".toByteArray())
            h.putInt(16)
            h.putShort(1)
            h.putShort(1)
            h.putInt(RATE)
            h.putInt(RATE * 2)
            h.putShort(2)
            h.putShort(16)
            h.put("data".toByteArray())
            h.putInt(dataBytes)
            out.write(h.array())
        }

        private fun fixWavHeader(f: File) {
            val size = f.length()
            if (size < 44) return
            java.io.RandomAccessFile(f, "rw").use { raf ->
                raf.seek(4)
                val total = java.nio.ByteBuffer.allocate(4).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                    .putInt((size - 8).toInt()).array()
                raf.write(total)
                raf.seek(40)
                val data = java.nio.ByteBuffer.allocate(4).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                    .putInt((size - 44).toInt()).array()
                raf.write(data)
            }
        }
    }
}
