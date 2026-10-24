package com.telecomfort.core

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Фоновый синк без FCM-сервера: раз в 15 мин тянет диалоги,
 * сравнивает unread с прошлым снимком и шлёт локальные уведомления.
 */
class SyncWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        try {
            val store = SecureStore(applicationContext)
            if (store.apiId.isBlank()) return@withContext Result.success()
            Core.call("init", mapOf(
                "dir" to applicationContext.filesDir.absolutePath,
                "api_id" to (store.apiId.toLongOrNull() ?: 0),
                "api_hash" to store.apiHash,
                "proxy" to store.proxy,
            ))
            val r = Core.call("dialogs")
            val arr = r.getJSONArray("result")
            val prev = applicationContext.getSharedPreferences("sync", Context.MODE_PRIVATE)
            val nm = applicationContext.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (Build.VERSION.SDK_INT >= 26) {
                nm.createNotificationChannel(NotificationChannel("msgs", "Сообщения", NotificationManager.IMPORTANCE_DEFAULT))
            }
            var nid = 1000
            for (i in 0 until arr.length()) {
                val c = arr.getJSONObject(i)
                val id = c.optLong("id")
                val unread = c.optInt("unread_count", 0)
                val was = prev.getInt("unread_$id", 0)
                if (unread > was && unread > 0) {
                    val n = NotificationCompat.Builder(applicationContext, "msgs")
                        .setSmallIcon(android.R.drawable.sym_action_chat)
                        .setContentTitle(c.optString("title", "?"))
                        .setContentText("Новых сообщений: ${unread - was}")
                        .setAutoCancel(true)
                        .build()
                    nm.notify(nid++, n)
                }
                prev.edit().putInt("unread_$id", unread).apply()
            }
            Result.success()
        } catch (e: Exception) {
            Result.retry()
        }
    }

    companion object {
        const val TAG = "telecomfort_sync"
        fun schedule(ctx: Context) {
            val req = PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
                .addTag(TAG)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(ctx).enqueueUniquePeriodicWork(TAG, ExistingPeriodicWorkPolicy.KEEP, req)
        }
        fun runOnce(ctx: Context) {
            WorkManager.getInstance(ctx).enqueue(OneTimeWorkRequestBuilder<SyncWorker>().addTag(TAG).build())
        }
    }
}
