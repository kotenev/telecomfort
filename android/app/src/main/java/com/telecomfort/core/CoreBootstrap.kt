package com.telecomfort.core

import android.content.Context

/**
 * Инициализация Rust-ядра вне Activity (сервис/ресивер/воркер):
 * поднимает сессию из EncryptedSharedPreferences, если ключи сохранены.
 */
object CoreBootstrap {
    @Volatile
    private var ready = false

    suspend fun ensure(ctx: Context): Boolean {
        if (ready) return true
        val store = SecureStore(ctx.applicationContext)
        val aid = store.apiId.toLongOrNull() ?: 0L
        if (aid == 0L || store.apiHash.isBlank()) return false
        Core.call(
            "init",
            mapOf(
                "dir" to ctx.applicationContext.filesDir.absolutePath,
                "api_id" to aid,
                "api_hash" to store.apiHash,
                "proxy" to store.proxy,
            ),
        )
        ready = true
        return true
    }

    /** Ядро уже инициализировано вручную (например, при автологине в AppNav). */
    fun markReady() {
        ready = true
    }

    /** После logout/смены аккаунта следующий ensure выполнит init заново. */
    fun reset() {
        ready = false
    }
}
