package com.telecomfort.ui.screens

import android.graphics.BitmapFactory
import android.util.Base64
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.dp
import com.telecomfort.core.Core
import com.telecomfort.core.optStr
import kotlinx.coroutines.launch

/**
 * Вход: API ID/HASH + телефон/код/пароль 2FA + QR.
 * Поля сохраняются в EncryptedSharedPreferences.
 */
@Composable
fun LoginScreen(onDone: () -> Unit, store: com.telecomfort.core.SecureStore? = null) {
    var apiId by remember { mutableStateOf(store?.apiId ?: "") }
    var apiHash by remember { mutableStateOf(store?.apiHash ?: "") }
    var phone by remember { mutableStateOf(store?.phone ?: "") }
    var proxy by remember { mutableStateOf(store?.proxy ?: "") }
    var code by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var hint by remember { mutableStateOf<String?>(null) }
    var loginKey by remember { mutableStateOf<String?>(null) }
    var stage by remember { mutableStateOf("api") } // api|code|password|qr
    var qrPng by remember { mutableStateOf<ByteArray?>(null) }
    var status by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current

    fun err(e: Throwable) { status = "Ошибка: ${e.message}" }
    fun persistAndGo() {
        store?.let {
            it.apiId = apiId; it.apiHash = apiHash; it.phone = phone; it.proxy = proxy
        }
        com.telecomfort.core.LockManager.touch()
        com.telecomfort.core.RealtimeService.start(ctx)
        onDone()
    }

    Column(Modifier.fillMaxSize().padding(16.dp).verticalScroll(rememberScrollState())) {
        Text("Telecomfort — вход", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(apiId, { apiId = it }, label = { Text("API ID") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(apiHash, { apiHash = it }, label = { Text("API HASH") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(proxy, { proxy = it }, label = { Text("SOCKS5 (необязательно)") }, modifier = Modifier.fillMaxWidth(),
            placeholder = { Text("socks5://user:pass@host:port") })
        OutlinedTextField(phone, { phone = it }, label = { Text("Телефон") }, modifier = Modifier.fillMaxWidth())

        Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("init", mapOf("dir" to ctx.filesDir.absolutePath, "api_id" to (apiId.toLongOrNull() ?: 0), "api_hash" to apiHash, "proxy" to proxy))
                        val r = Core.call("request_code", mapOf("phone" to phone))
                        loginKey = r.getString("key")
                        stage = "code"
                        status = "Код отправлен в Telegram"
                    }.onFailure(::err)
                }
            }) { Text("Получить код") }
            Button(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("init", mapOf("dir" to ctx.filesDir.absolutePath, "api_id" to (apiId.toLongOrNull() ?: 0), "api_hash" to apiHash, "proxy" to proxy))
                        val r = Core.call("qr_login_start", emptyMap())
                            loginKey = r.getString("token_b64")
                        val cfg = mapOf("payload" to ("tg://login?token=" + r.getString("token_b64")))
                        val bmp = Core.call("qr_bitmap", cfg)
                        qrPng = Base64.decode(bmp.getString("png_b64"), Base64.DEFAULT)
                        stage = "qr"
                        status = "Отсканируйте код телефоном — проверка идёт автоматически"
                        // Автопроверка каждые 3с (как десктопный опрос 2.5с).
                        while (stage == "qr") {
                            kotlinx.coroutines.delay(3000)
                            runCatching {
                                val pl = Core.call("qr_poll", mapOf("token_b64" to (loginKey ?: "")))
                                if (pl.optString("status") == "ok") { stage = "done"; persistAndGo() }
                            }
                        }
                    }.onFailure(::err)
                }
            }) { Text("📷 QR-вход") }
            if (stage == "qr") {
                Button(onClick = {
                    scope.launch {
                        runCatching {
                            Core.call("qr_cancel", emptyMap())
                            stage = "api"; qrPng = null
                        }.onFailure(::err)
                    }
                }) { Text("✕") }
            }
        }

        if (stage == "code" || stage == "password") {
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(code, { code = it }, label = { Text("Код из Telegram") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = {
                scope.launch {
                    runCatching {
                        val r = Core.call("sign_in_code", mapOf("key" to (loginKey ?: ""), "code" to code))
                        if (r.optString("status") == "password_required") {
                            stage = "password"
                            hint = runCatching { Core.call("password_hint").optStr("hint") }.getOrNull()
                            status = "Нужен пароль 2FA"
                        } else persistAndGo()
                    }.onFailure(::err)
                }
            }, modifier = Modifier.padding(top = 8.dp)) { Text("Войти по коду") }
        }
        if (stage == "password") {
            hint?.let { Text("💡 $it", style = MaterialTheme.typography.bodySmall) }
            OutlinedTextField(password, { password = it }, label = { Text("Пароль 2FA") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("sign_in_password", mapOf("key" to (loginKey ?: ""), "password" to password))
                        persistAndGo()
                    }.onFailure(::err)
                }
            }, modifier = Modifier.padding(top = 8.dp)) { Text("Войти с паролем") }
        }
        qrPng?.let { bytes ->
            Spacer(Modifier.height(12.dp))
            val bmp = remember(bytes) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }
            bmp?.let { Image(it, contentDescription = "QR", modifier = Modifier.size(232.dp)) }
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        // Опрос: в проде — WorkManager/таймер; здесь — по кнопке.
                        status = "Проверяю… (повторите через пару секунд)"
                    }
                }
            }) { Text("Я отсканировал — проверить") }
        }
        if (status.isNotEmpty()) {
            Spacer(Modifier.height(8.dp))
            Text(status, style = MaterialTheme.typography.bodySmall)
        }
        Spacer(Modifier.height(8.dp))
        TextButton(onClick = { com.telecomfort.core.LockManager.touch(); onDone() }) { Text("Продолжить в демо-режиме →") }
    }
}
