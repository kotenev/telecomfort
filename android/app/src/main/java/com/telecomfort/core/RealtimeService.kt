package com.telecomfort.core

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject

/**
 * Замена FCM: foreground-сервис держит MTProto-поток обновлений
 * (`poll_updates`, long-poll 25 с) и показывает локальные уведомления.
 *
 * Работает без Firebase/сторонних серверов: события приходят напрямую
 * от Telegram по уже авторизованной сессии. При смерти процесса
 * перезапускается (START_STICKY) и восстанавливает init из SecureStore.
 */
class RealtimeService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    @Volatile
    private var running = true

    override fun onCreate() {
        super.onCreate()
        startForeground(NOTIF_ID, buildServiceNotification())
        ensureChannels()
        scope.launch { loop() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        running = false
        scope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private suspend fun loop() {
        var backoff = 0L
        while (running && scope.isActive) {
            try {
                if (!CoreBootstrap.ensure(this)) {
                    delay(30_000)
                    continue
                }
                val r = Core.call("poll_updates", mapOf("timeout_ms" to 25_000))
                val u = r.optJSONObject("update")
                if (u != null) {
                    backoff = 0
                    handle(u)
                } else {
                    delay(200)
                }
            } catch (_: Exception) {
                backoff = (backoff + 2_000).coerceAtMost(60_000)
                delay(backoff)
            }
        }
    }

    private fun handle(u: JSONObject) {
        when (u.optString("kind")) {
            "message" -> {
                if (u.optBoolean("outgoing", false) || AppVisibility.visible) return
                val text = u.optString("text").ifBlank { "[медиа]" }
                val title = u.optString("sender").ifBlank { "Telegram" }
                notify("msg", title, text.take(200), openAppIntent(), low = false)
            }
            "edit" -> Unit // правки не пушим
            "delete" -> Unit
            "call" -> {
                val video = if (u.optBoolean("video")) " (видео)" else ""
                val decline = PendingIntent.getBroadcast(
                    this, 0,
                    Intent(this, CallActionReceiver::class.java).apply {
                        action = CallActionReceiver.ACTION_DECLINE
                        putExtra("call_id", u.optLong("call_id"))
                        putExtra("access_hash", u.optLong("call_access_hash"))
                    },
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                )
                val n = NotificationCompat.Builder(this, "calls")
                    .setSmallIcon(android.R.drawable.sym_call_incoming)
                    .setContentTitle("Входящий звонок$video")
                    .setContentText("От ${
                        u.optString("chat_id").ifBlank { "пользователя" }
                    } — принять можно в официальном клиенте")
                    .setPriority(NotificationCompat.PRIORITY_HIGH)
                    .setCategory(NotificationCompat.CATEGORY_CALL)
                    .setAutoCancel(true)
                    .setContentIntent(openAppIntent())
                    .addAction(0, "Отклонить", decline)
                    .build()
                nm().notify(CALL_NOTIF_ID, n)
            }
            "call_ended" -> {
                if (u.optBoolean("missed")) {
                    notify("calls", "Пропущенный звонок", "Звонок отменён или пропущен", openAppIntent(), low = false)
                }
                nm().cancel(CALL_NOTIF_ID)
            }
        }
    }

    private fun openAppIntent(): PendingIntent = PendingIntent.getActivity(
        this, 0,
        Intent(this, Class.forName("com.telecomfort.MainActivity")),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    private fun nm() = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    private fun ensureChannels() {
        if (Build.VERSION.SDK_INT < 26) return
        val nm = nm()
        nm.createNotificationChannel(
            NotificationChannel("msgs", "Сообщения", NotificationManager.IMPORTANCE_DEFAULT)
        )
        nm.createNotificationChannel(
            NotificationChannel("calls", "Звонки", NotificationManager.IMPORTANCE_HIGH)
        )
        nm.createNotificationChannel(
            NotificationChannel("service", "Фоновые уведомления", NotificationManager.IMPORTANCE_MIN)
        )
    }

    private fun notify(channel: String, title: String, text: String, pi: PendingIntent, low: Boolean) {
        val n = NotificationCompat.Builder(this, channel)
            .setSmallIcon(android.R.drawable.sym_action_chat)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(pi)
            .setAutoCancel(true)
            .setPriority(if (low) NotificationCompat.PRIORITY_LOW else NotificationCompat.PRIORITY_DEFAULT)
            .build()
        nm().notify(System.currentTimeMillis().toInt(), n)
    }

    private fun buildServiceNotification(): android.app.Notification =
        NotificationCompat.Builder(this, "service")
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle("Telecomfort")
            .setContentText("Уведомления о новых сообщениях включены")
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .build()

    companion object {
        private const val NOTIF_ID = 42
        private const val CALL_NOTIF_ID = 43

        fun start(ctx: Context) {
            val i = Intent(ctx, RealtimeService::class.java)
            if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(i) else ctx.startService(i)
        }

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, RealtimeService::class.java))
        }
    }
}
