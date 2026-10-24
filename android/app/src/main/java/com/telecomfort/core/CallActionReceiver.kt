package com.telecomfort.core

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import kotlinx.coroutines.runBlocking
import kotlin.concurrent.thread

/** Действие «Отклонить» на уведомлении входящего звонка. */
class CallActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_DECLINE) return
        val id = intent.getLongExtra("call_id", 0L)
        val hash = intent.getLongExtra("access_hash", 0L)
        val pending = goAsync()
        thread(name = "decline-call") {
            try {
                runBlocking {
                    if (CoreBootstrap.ensure(context)) {
                        Core.call("decline_call", mapOf("call_id" to id, "access_hash" to hash))
                    }
                }
            } catch (_: Exception) {
                // звонок уже завершён/устарел — молча
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_DECLINE = "com.telecomfort.DECLINE_CALL"
    }
}
