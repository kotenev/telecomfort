package com.telecomfort.core

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * Мост к Rust-ядру: один нативный вызов nativeCall(cmdJson): String.
 * Ответ всегда {"ok":true,...} или {"ok":false,"error":"..."}.
 * Все вызовы — на Dispatchers.IO (JNI блокирует поток).
 */
object Core {
    init {
        // libtgvoip.so — нативный VoIP-стек (см. android/build-tgvoip.sh).
        System.loadLibrary("tgvoip")
        System.loadLibrary("telecomfort")
    }

    @JvmStatic
    private external fun nativeCall(cmdJson: String): String?

    suspend fun call(op: String, params: Map<String, Any?> = emptyMap()): JSONObject =
        withContext(Dispatchers.IO) {
            val req = JSONObject().put("op", op)
            for ((k, v) in params) {
                when (v) {
                    null -> req.put(k, JSONObject.NULL)
                    is List<*> -> req.put(k, JSONArray(v))
                    is Long, is Int, is Boolean, is Double, is String -> req.put(k, v)
                    else -> req.put(k, v.toString())
                }
            }
            val raw = nativeCall(req.toString()) ?: throw IllegalStateException("nativeCall вернул null")
            JSONObject(raw).also {
                if (!it.optBoolean("ok", false)) {
                    throw IllegalStateException(it.optString("error", "bridge error"))
                }
            }
        }

    // --- удобные обёртки (тонкий слой поверх JSON-протокола) ---
    suspend fun dialogs(): List<Chat> = call("dialogs").array("result").map(::parseChat)
    suspend fun history(chat: Long, limit: Int = 100): List<Message> =
        call("history", mapOf("chat" to chat, "limit" to limit)).array("result").map(::parseMessage)

    private fun JSONObject.array(key: String): List<JSONObject> {
        val a = optJSONArray(key) ?: return emptyList()
        return (0 until a.length()).map { a.getJSONObject(it) }
    }
}
