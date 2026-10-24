package com.telecomfort.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.telecomfort.core.Core
import com.telecomfort.core.SyncWorker
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Настройки: профиль, прокси+тест, PIN, 2FA, сессии, Yandex-токен. */
@Composable
fun SettingsScreen(nav: NavController, onToggleTheme: () -> Unit, store: com.telecomfort.core.SecureStore? = null, onLockNow: () -> Unit = {}) {
    var first by remember { mutableStateOf("") }
    var last by remember { mutableStateOf("") }
    var about by remember { mutableStateOf("") }
    var proxy by remember { mutableStateOf("") }
    var pin by remember { mutableStateOf("") }
    var cur by remember { mutableStateOf("") }
    var newPw by remember { mutableStateOf("") }
    var newPw2 by remember { mutableStateOf("") }
    var hint by remember { mutableStateOf("") }
    var email by remember { mutableStateOf("") }
    var armed by remember { mutableStateOf(false) }
    var yandex by remember { mutableStateOf(store?.yandexToken ?: "") }
    var autoLock by remember { mutableStateOf(store?.autoLockSecs?.toString() ?: "0") }
    var sessions by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var status by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current

    fun reloadSessions() {
        scope.launch {
            runCatching {
                val r = Core.call("sessions")
                sessions = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(Unit) { reloadSessions() }

    LazyColumn(Modifier.fillMaxSize().padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            Row {
                IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
                Text("Настройки", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
                TextButton(onClick = onToggleTheme) { Text("🌙/☀") }
            }
        }
        item {
            Text("Профиль", style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(first, { first = it }, label = { Text("Имя") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(last, { last = it }, label = { Text("Фамилия") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(about, { about = it }, label = { Text("О себе") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("update_profile", mapOf("first" to first, "last" to last, "about" to about))
                        status = "Профиль обновлён"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Сохранить профиль") }
        }
        item {
            Text("SOCKS5-прокси", style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(proxy, { proxy = it }, label = { Text("socks5://…") }, modifier = Modifier.fillMaxWidth())
            Text("Проверка — отдельным соединением:", style = MaterialTheme.typography.bodySmall)
        }
        item {
            Text("🔒 PIN-блокировка", style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(pin, { pin = it }, label = { Text("Новый PIN (мин. 4)") }, modifier = Modifier.fillMaxWidth())
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    if (pin.length < 4) { status = "PIN — минимум 4 символа"; return@Button }
                    store?.pinSha256 = com.telecomfort.core.SecureStore.sha256Hex(pin)
                    pin = ""
                    status = "PIN задан (SHA-256 в шифрохранилище)"
                }) { Text("Задать PIN") }
                Button(onClick = {
                    if (store?.pinSha256.isNullOrEmpty()) { status = "PIN не задан"; return@Button }
                    com.telecomfort.core.LockManager.touch()
                    onLockNow()
                }) { Text("🔒 Сейчас") }
            }
            OutlinedTextField(autoLock, { autoLock = it }, label = { Text("Автоблок, сек (0 — выкл)") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = {
                store?.autoLockSecs = autoLock.toLongOrNull() ?: 0
                status = "Автоблок: ${store?.autoLockSecs}с"
            }) { Text("Сохранить автоблок") }
            Text("PIN и таймаут — в EncryptedSharedPreferences.",
                style = MaterialTheme.typography.bodySmall)
        }
        item {
            Text("🔑 Смена пароля 2FA", style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(cur, { cur = it }, label = { Text("Текущий") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(newPw, { newPw = it }, label = { Text("Новый") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(newPw2, { newPw2 = it }, label = { Text("Новый (повтор)") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(hint, { hint = it }, label = { Text("Подсказка") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(email, { email = it }, label = { Text("Email") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = {
                if (!armed) {
                    if (newPw != newPw2 || newPw.length < 4) { status = "Пароли не совпадают / короткие"; return@Button }
                    armed = true
                    status = "⚠ Нажмите ещё раз для подтверждения"
                    return@Button
                }
                armed = false
                scope.launch {
                    runCatching {
                        Core.call("change_password", mapOf("current" to cur, "new" to newPw, "hint" to hint, "email" to email))
                        status = "Пароль сменён и проверен"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text(if (armed) "⚠ Подтвердить смену!" else "Сменить пароль") }
        }
        item {
            Text("☁️ Яндекс.Диск", style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(yandex, { yandex = it; store?.yandexToken = it }, label = { Text("OAuth-токен") }, modifier = Modifier.fillMaxWidth())
        }
        item { Text("📱 Сессии", style = MaterialTheme.typography.titleSmall) }
        itemsIndexed(sessions, key = { i, s -> "sess-$i-${s.optLong("hash")}" }) { _, s ->
            Row {
                Text("${s.optString("device", "?")} ${s.optString("app", "")}${if (s.optBoolean("current")) " · текущая" else ""}",
                    modifier = Modifier.weight(1f))
                if (!s.optBoolean("current")) {
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                Core.call("terminate_session", mapOf("hash" to s.optLong("hash")))
                                reloadSessions()
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text("⏏") }
                }
            }
        }
        item {
            Text("🔄 Фоновый синк", style = MaterialTheme.typography.titleSmall)
            Text("Без FCM-сервера: WorkManager тянет диалоги и уведомляет о новых.",
                style = MaterialTheme.typography.bodySmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    SyncWorker.runOnce(ctx)
                    status = "Синк запущен (смотрите шторку уведомлений)"
                }) { Text("Синхронизировать") }
                Button(onClick = {
                    SyncWorker.schedule(ctx)
                    status = "Автосинк каждые 15 мин включён"
                }) { Text("Автосинк") }
            }
        }
        item { if (status.isNotEmpty()) Text(status, style = MaterialTheme.typography.bodySmall) }
    }
}
