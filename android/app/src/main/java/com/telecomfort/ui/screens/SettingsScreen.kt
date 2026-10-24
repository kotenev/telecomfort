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
fun SettingsScreen(nav: NavController, onToggleTheme: () -> Unit, store: com.telecomfort.core.SecureStore? = null, onLockNow: () -> Unit = {}, onThemeMode: (String) -> Unit = {}, currentTheme: String = "system", onLogout: () -> Unit = {}) {
    var first by remember { mutableStateOf("") }
    var last by remember { mutableStateOf("") }
    var about by remember { mutableStateOf("") }
    var showLogout by remember { mutableStateOf(false) }
    var proxy by remember { mutableStateOf(store?.proxy ?: "") }
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
    var uname by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var emojiList by remember { mutableStateOf<List<Long>>(emptyList()) }
    var showEmoji by remember { mutableStateOf(false) }
    var unameMsg by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    // SAF-диалоги экспорта/импорта session-файла (мульти-девайс).
    val exportSession = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.CreateDocument("application/octet-stream")
    ) { uri ->
        if (uri != null) scope.launch {
            runCatching {
                val src = java.io.File(ctx.filesDir, "session_android.session")
                ctx.contentResolver.openOutputStream(uri)?.use { out ->
                    src.inputStream().use { it.copyTo(out) }
                }
                status = "Сессия экспортирована"
            }.onFailure { status = "Ошибка экспорта: ${it.message}" }
        }
    }
    val importSession = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.OpenDocument()
    ) { uri ->
        if (uri != null) scope.launch {
            runCatching {
                val dst = java.io.File(ctx.filesDir, "session_android.session")
                ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                status = "Сессия импортирована — перезапустите приложение"
            }.onFailure { status = "Ошибка импорта: ${it.message}" }
        }
    }
    var avatarPath by remember { mutableStateOf<String?>(null) }
    val pickAvatar = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.GetContent()
    ) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        val dst = java.io.File(ctx.cacheDir, "avatar_${System.currentTimeMillis()}.jpg")
        ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
        avatarPath = dst.absolutePath
        status = "Файл выбран: ${dst.name} — нажмите «📷 Загрузить аватар»"
    }

    fun reloadSessions() {
        scope.launch {
            runCatching {
                val r = Core.call("sessions")
                sessions = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(Unit) {
        reloadSessions()
        scope.launch {
            runCatching {
                val p = Core.call("get_profile", emptyMap())
                first = p.optString("first_name", "")
                last = p.optString("last_name", "")
                about = p.optString("about", "")
                uname = p.optString("username", "")
            }
        }
    }

    LazyColumn(Modifier.fillMaxSize().padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            Row {
                IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
                Text("Настройки", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
                Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text("Тема: ", style = MaterialTheme.typography.bodySmall)
                        listOf("system" to "Авто", "dark" to "🌙", "light" to "☀").forEach { (mode, label) ->
                            FilterChip(mode == currentTheme, { onThemeMode(mode) }, { Text(label) })
                        }
                    }
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
            OutlinedTextField(uname, { uname = it }, label = { Text("username (без @)") }, modifier = Modifier.fillMaxWidth())
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    scope.launch {
                        runCatching {
                            val r = Core.call("check_username", mapOf("username" to uname))
                            unameMsg = if (r.optBoolean("free")) "свободен ✓" else "занят"
                        }.onFailure { unameMsg = "Ошибка: ${it.message}" }
                    }
                }) { Text("Проверить") }
                Button(onClick = {
                    scope.launch {
                        runCatching {
                            Core.call("update_username", mapOf("username" to uname))
                            unameMsg = "username обновлён"
                        }.onFailure { unameMsg = "Ошибка: ${it.message}" }
                    }
                }) { Text("Занять") }
            }
            if (unameMsg.isNotEmpty()) Text(unameMsg, style = MaterialTheme.typography.bodySmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    if (avatarPath == null) pickAvatar.launch("image/*")
                    else scope.launch {
                        runCatching {
                            Core.call("upload_avatar", mapOf("path" to (avatarPath ?: "")))
                            avatarPath = null
                            status = "Аватар загружен"
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }) { Text(if (avatarPath == null) "📷 Выбрать файл" else "📷 Загрузить аватар") }
                Button(onClick = {
                    scope.launch {
                        runCatching {
                            Core.call("set_emoji_status", mapOf("doc" to JSONObject.NULL))
                            status = "Эмодзи-статус снят"
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }) { Text("😀 Статус: снять") }
            }
            Text("Аватар: выберите файл ниже (jpg/png).", style = MaterialTheme.typography.bodySmall)
        }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = { exportSession.launch("session_android.session") }) { Text("⤴ Экспорт сессии") }
                TextButton(onClick = { importSession.launch(arrayOf("*/*")) }) { Text("⤵ Импорт сессии") }
            }
        }
        item {
            Text("SOCKS5-прокси", style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(proxy, { proxy = it }, label = { Text("socks5://…") }, modifier = Modifier.fillMaxWidth())
            Row {
                TextButton(onClick = {
                    store?.proxy = proxy
                    status = "Прокси сохранён"
                }) { Text("💾") }
                TextButton(onClick = {
                    scope.launch {
                        runCatching {
                            val r = Core.call("test_proxy", mapOf("api_id" to (store?.apiId?.toLongOrNull() ?: 0), "api_hash" to (store?.apiHash ?: ""), "proxy" to proxy))
                            status = "Прокси: соединение OK"
                        }.onFailure { status = "Прокси: ${it.message}" }
                    }
                }) { Text("🔌 Проверить") }
                Text("отдельным соединением", style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.align(androidx.compose.ui.Alignment.CenterVertically))
            }
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
                Button(onClick = {
                    store?.pinSha256 = ""
                    status = "PIN убран"
                }) { Text("Убрать PIN") }
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
        item {
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Text("📱 Сессии", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                TextButton(onClick = {
                    scope.launch {
                        runCatching {
                            Core.call("terminate_others", emptyMap())
                            status = "Другие сессии завершены"
                            reloadSessions()
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }) { Text("Завершить другие") }
            }
        }
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
        item {
            var showAbout by remember { mutableStateOf(false) }
            TextButton(onClick = { showAbout = true }) { Text("ℹ️ О приложении") }
            if (showAbout) {
                AlertDialog(onDismissRequest = { showAbout = false }, title = { Text("Telecomfort") }, text = {
                    Text("Нативный Telegram-клиент: общий Rust-core (MTProto, медиа, AI-детект) + Kotlin/Compose UI.\nВерсия 0.1.0 (debug).\n\nКопирование из защищённых чатов работает: флаг noforwards — клиентский, сервер отдаёт данные.")
                }, confirmButton = { TextButton(onClick = { showAbout = false }) { Text("Закрыть") } })
            }
        }
        item {
            Text("😀 Эмодзи-статус", style = MaterialTheme.typography.titleSmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    scope.launch {
                        runCatching {
                            val r = Core.call("default_emoji_statuses", emptyMap())
                            val arr = r.getJSONArray("result")
                            emojiList = (0 until arr.length()).map { arr.optLong(it) }
                            showEmoji = true
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }) { Text("Выбрать") }
                Button(onClick = {
                    scope.launch {
                        runCatching {
                            Core.call("set_emoji_status", mapOf("doc" to JSONObject.NULL))
                            status = "Эмодзи-статус снят"
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }) { Text("Снять") }
            }
        }
        item {
        if (showEmoji) {
            AlertDialog(onDismissRequest = { showEmoji = false }, title = { Text("😀 Статус") }, text = {
                LazyColumn(Modifier.heightIn(max = 300.dp)) {
                    items(emojiList.size) { i ->
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Text("custom_emoji:${emojiList[i]}", modifier = Modifier.weight(1f))
                            TextButton(onClick = {
                                scope.launch {
                                    kotlin.runCatching {
                                        Core.call("set_emoji_status", mapOf("doc" to emojiList[i]))
                                        showEmoji = false
                                        status = "Статус установлен"
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("OK") }
                        }
                    }
                }
            }, confirmButton = { TextButton(onClick = { showEmoji = false }) { Text("Закрыть") } })
        }
        }
        item {
            TextButton(onClick = { showLogout = true }, modifier = Modifier.fillMaxWidth()) {
                Text("🚪 Выйти из аккаунта", color = MaterialTheme.colorScheme.error)
            }
        }
        item { if (status.isNotEmpty()) Text(status, style = MaterialTheme.typography.bodySmall) }
    }
    if (showLogout) {
        AlertDialog(onDismissRequest = { showLogout = false }, title = { Text("🚪 Выйти?") }, text = {
            Text("Сессия на устройстве будет удалена. Переписка на сервере сохранится.")
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("logout", emptyMap())
                        showLogout = false
                        com.telecomfort.core.LockManager.locked = false
                        onLogout()
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Выйти", color = MaterialTheme.colorScheme.error) }
        }, dismissButton = { TextButton(onClick = { showLogout = false }) { Text("Отмена") } })
    }
}
