package com.telecomfort.core

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.security.MessageDigest

/** Шифрохранилище (EncryptedSharedPreferences): ключи, прокси, PIN-sha256, токены. */
class SecureStore(ctx: Context) {
    private val prefs: SharedPreferences by lazy {
        val masterKey = MasterKey.Builder(ctx).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build()
        EncryptedSharedPreferences.create(
            ctx, "telecomfort_secure", masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    var apiId: String
        get() = prefs.getString("api_id", "") ?: ""
        set(v) = prefs.edit().putString("api_id", v).apply()
    var apiHash: String
        get() = prefs.getString("api_hash", "") ?: ""
        set(v) = prefs.edit().putString("api_hash", v).apply()
    var phone: String
        get() = prefs.getString("phone", "") ?: ""
        set(v) = prefs.edit().putString("phone", v).apply()
    var proxy: String
        get() = prefs.getString("proxy", "") ?: ""
        set(v) = prefs.edit().putString("proxy", v).apply()
    var yandexToken: String
        get() = prefs.getString("yandex", "") ?: ""
        set(v) = prefs.edit().putString("yandex", v).apply()
    var pinSha256: String
        get() = prefs.getString("pin", "") ?: ""
        set(v) = prefs.edit().putString("pin", v).apply()
    var autoLockSecs: Long
        get() = prefs.getLong("autolock", 0)
        set(v) = prefs.edit().putLong("autolock", v).apply()

    companion object {
        fun sha256Hex(s: String): String {
            val d = MessageDigest.getInstance("SHA-256").digest(s.toByteArray())
            return d.joinToString("") { "%02x".format(it) }
        }
    }
}

/** Глобальный менеджер блокировки: touch() при действиях, проверка таймаута. */
object LockManager {
    var locked: Boolean = false
    var lastActive: Long = System.currentTimeMillis()
    fun touch() { lastActive = System.currentTimeMillis() }
    fun due(autoLockSecs: Long): Boolean {
        if (autoLockSecs <= 0) return false
        return System.currentTimeMillis() - lastActive >= autoLockSecs * 1000
    }
}
