package com.telecomfort.ui.screens

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.navigation.NavController
import com.telecomfort.core.Core
import com.telecomfort.core.Message
import com.telecomfort.core.parseMessage
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

/**
 * Чат: история, композер (текст/файлы/войс/кружок/стикеры/GIF/гео),
 * TTL, экспорт, инфо, реакции+кто, прыжок к дате.
 * Защита от копирования НЕ действует: текст/медиа доступны (см. мост).
 */
@Composable
fun ChatScreen(chatId: Long, nav: NavController) {
    var items by remember { mutableStateOf<List<Message>>(emptyList()) }
    var rawItems by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var text by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var showInfo by remember { mutableStateOf(false) }
    var showParticipants by remember { mutableStateOf(false) }
    var participants by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var showScheduled by remember { mutableStateOf(false) }
    var scheduled by remember { mutableStateOf<List<Message>>(emptyList()) }
    var showPinned by remember { mutableStateOf(false) }
    var pinned by remember { mutableStateOf<List<Message>>(emptyList()) }
    var showPoll by remember { mutableStateOf(false) }
    var pollQ by remember { mutableStateOf("") }
    var pollOpts by remember { mutableStateOf("") }
    var infoJson by remember { mutableStateOf<JSONObject?>(null) }
    var showTtl by remember { mutableStateOf(false) }
    var showExport by remember { mutableStateOf(false) }
    var showStickers by remember { mutableStateOf(false) }
    var showGifs by remember { mutableStateOf(false) }
    var stickers by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var gifs by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var jumpDate by remember { mutableStateOf("") }
    var recording by remember { mutableStateOf<com.telecomfort.core.PcmRecorder?>(null) }
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    val listState = rememberLazyListState()

    fun reload(limit: Int = 100) {
        scope.launch {
            runCatching {
                val r = Core.call("history", mapOf("chat" to chatId, "limit" to limit))
                rawItems = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                items = rawItems.map { parseMessage(it) }
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(chatId) { reload() }

    val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            runCatching {
                // Копируем в кэш приложения (без SAF-сложностей).
                val name = "attach_${System.currentTimeMillis()}"
                val dst = File(ctx.cacheDir, name)
                ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                val r = Core.call("send_file", mapOf("chat" to chatId, "path" to dst.absolutePath, "caption" to text))
                status = "Отправлено (msg ${r.optInt("id")})"
                text = ""
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }

    fun sendText() {
        scope.launch {
            runCatching {
                Core.call("send_text", mapOf("chat" to chatId, "text" to text))
                text = ""
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }

    fun toggleRecord() {
        val rec = recording
        if (rec != null) {
            // Стоп → WAV (PCM 48кГц моно) → OGG/Opus → войс.
            recording = null
            val wav = runCatching { rec.stop() }.getOrNull() ?: run {
                status = "Запись не удалась"
                return
            }
            scope.launch {
                runCatching {
                    val ogg = File(ctx.cacheDir, "voice_${System.currentTimeMillis()}.ogg")
                    Core.call("wav_to_ogg", mapOf("src" to wav.absolutePath, "dst" to ogg.absolutePath))
                    Core.call("send_voice", mapOf("chat" to chatId, "path" to ogg.absolutePath, "caption" to text))
                    status = "Войс отправлен (Opus)"
                    text = ""
                    reload()
                }.onFailure { status = "Ошибка: ${it.message}" }
            }
            return
        }
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            status = "Нужно разрешение на микрофон"
            return
        }
        recording = try {
            com.telecomfort.core.PcmRecorder.start(File(ctx.cacheDir, "rec_${System.currentTimeMillis()}.wav"))
        } catch (e: Exception) {
            status = "Микрофон: ${e.message}"
            null
        }
        if (recording != null) status = "● Запись… нажмите ■ для отправки"
    }

    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().padding(4.dp)
                .horizontalScroll(androidx.compose.foundation.rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        val full = Core.call("chat_full", mapOf("chat" to chatId))
                        infoJson = full
                        showInfo = true
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("ℹ️") }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        val r = Core.call("participants", mapOf("chat" to chatId, "limit" to 200))
                        participants = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                        showParticipants = true
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("👥") }
            IconButton(onClick = { showTtl = true }) { Text("⏳") }
            var showJump by remember { mutableStateOf(false) }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        val r = Core.call("scheduled_list", mapOf("chat" to chatId))
                        scheduled = (0 until r.getJSONArray("result").length()).map { parseMessage(r.getJSONArray("result").getJSONObject(it)) }
                        showScheduled = true
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("⏰") }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        val r = Core.call("pinned_list", mapOf("chat" to chatId, "limit" to 50))
                        pinned = (0 until r.getJSONArray("result").length()).map { parseMessage(r.getJSONArray("result").getJSONObject(it)) }
                        showPinned = true
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("📌") }
            IconButton(onClick = { showPoll = true }) { Text("📊") }
            IconButton(onClick = { jumpDate = ""; showJump = true }) { Text("📅") }
            if (showJump) {
                AlertDialog(onDismissRequest = { showJump = false }, title = { Text("📅 Перейти к дате") }, text = {
                    OutlinedTextField(jumpDate, { jumpDate = it }, label = { Text("ГГГГ-ММ-ДД") })
                }, confirmButton = {
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                val parts = jumpDate.trim().split("-")
                                require(parts.size == 3)
                                val cal = java.util.Calendar.getInstance(java.util.TimeZone.getTimeZone("UTC"))
                                cal.set(parts[0].toInt(), parts[1].toInt() - 1, parts[2].toInt(), 12, 0, 0)
                                val r = Core.call("history_at_date", mapOf("chat" to chatId, "date" to cal.timeInMillis / 1000, "limit" to 100))
                                items = (0 until r.getJSONArray("items").length()).map { parseMessage(r.getJSONArray("items").getJSONObject(it)) }
                                status = "Прыжок: показано ${items.size}"
                                showJump = false
                            }.onFailure { status = "Дата — ГГГГ-ММ-ДД" }
                        }
                    }) { Text("Перейти") }
                }, dismissButton = { TextButton(onClick = { showJump = false }) { Text("Отмена") } })
            }
            IconButton(onClick = { showExport = true }) { Text("⤵") }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        LazyColumn(Modifier.weight(1f), state = listState, reverseLayout = true) {
            itemsIndexed(items.reversed(), key = { _, m -> m.id }) { ri, m ->
                    val poll = rawItems.getOrNull(rawItems.size - 1 - ri)?.optJSONObject("poll")
                Card(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 2.dp)) {
                    Column(Modifier.padding(8.dp)) {
                        Text(if (m.outgoing) "я" else (m.sender ?: "—"), style = MaterialTheme.typography.labelSmall)
                        Text(m.text)
                        if (m.hasMedia) {
                            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("[${m.mediaKind ?: "медиа"} ${m.fileName ?: ""}]", style = MaterialTheme.typography.bodySmall)
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            val r = Core.call("download_media", mapOf("chat" to chatId, "id" to m.id))
                                            status = "Скачано: ${r.optString("path")}"
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("⬇") }
                            }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        Core.call("react", mapOf("chat" to chatId, "id" to m.id, "emoji" to "❤"))
                                        reload()
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("❤") }
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        val r = Core.call("reactions", mapOf("chat" to chatId, "id" to m.id))
                                        val names = (0 until r.getJSONArray("result").length())
                                            .map { r.getJSONArray("result").getJSONObject(it).optString("name") }
                                        status = "👥 " + names.joinToString(", ")
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("👥") }
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        Core.call("report_spam", mapOf("chat" to chatId))
                                        status = "Жалоба отправлена"
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("⚠") }
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        val r = Core.call("translate", mapOf("chat" to chatId, "id" to m.id, "to" to "ru"))
                                        status = "Перевод: " + r.optString("text").take(300)
                                    }.onFailure { status = "Ошибка перевода (нужен Premium?): ${it.message}" }
                                }
                            }) { Text("🌐") }
                        if (poll != null) {
                            Column {
                                Text("📊 ${poll.optString("question")}", style = MaterialTheme.typography.bodySmall)
                                val answers = poll.optJSONArray("answers") ?: org.json.JSONArray()
                                for (ai in 0 until answers.length()) {
                                    val a = answers.getJSONObject(ai)
                                    TextButton(onClick = {
                                        scope.launch {
                                            runCatching {
                                                Core.call("vote", mapOf("chat" to chatId, "id" to m.id, "option" to a.optString("option")))
                                                reload()
                                            }.onFailure { status = "Ошибка: ${it.message}" }
                                        }
                                    }) {
                                        Text("${if (a.optBoolean("chosen")) "✓ " else ""}${a.optString("text")} ${a.optInt("votes", 0)}")
                                    }
                                }
                            }
                        }
                        }
                    }
                }
            }
        }
        if (showStickers || showGifs) {
            Row(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                (if (showStickers) stickers else gifs).take(12).forEach { s ->
                    val emoji = s.optString("emoji", if (showStickers) "😀" else "🎞")
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                val fr = s.optString("file_reference", "")
                                Core.call("send_doc_ref", mapOf(
                                    "chat" to chatId, "doc_id" to s.optLong("id"),
                                    "access_hash" to s.optLong("access_hash"), "file_ref" to fr,
                                ))
                                showStickers = false; showGifs = false
                                reload()
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text(emoji.ifEmpty { "😀" }) }
                }
                Spacer(Modifier.weight(1f))
                TextButton(onClick = { showStickers = false; showGifs = false }) { Text("✕") }
            }
        }
        Row(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            OutlinedTextField(text, { text = it }, modifier = Modifier.weight(1f), placeholder = { Text("Сообщение…") })
            IconButton(onClick = { sendText() }) { Icon(Icons.Default.Send, "Отправить") }
            IconButton(onClick = { pickFile.launch("*/*") }) { Text("📎") }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("save_draft", mapOf("chat" to chatId, "text" to text))
                        status = "Черновик сохранён (синк)"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("💾") }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        // Геопозиция: центр Москвы по умолчанию; TODO: GPS.
                        Core.call("send_geo", mapOf("chat" to chatId, "lat" to 55.75, "long" to 37.61))
                        reload()
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("📍") }
            IconButton(onClick = { toggleRecord() }) { Text(if (recording != null) "■" else "🎙") }
            IconButton(onClick = {
                showStickers = !showStickers; showGifs = false
                if (showStickers && stickers.isEmpty()) scope.launch {
                    runCatching {
                        val r = Core.call("recent_stickers", emptyMap())
                        stickers = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                    }
                }
            }) { Text("😀") }
            IconButton(onClick = {
                showGifs = !showGifs; showStickers = false
                if (showGifs && gifs.isEmpty()) scope.launch {
                    runCatching {
                        val r = Core.call("saved_gifs", emptyMap())
                        gifs = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                    }
                }
            }) { Text("🎞") }
        }
        if (status.isNotEmpty()) Text(status, modifier = Modifier.padding(8.dp), style = MaterialTheme.typography.bodySmall)
    }

    if (showInfo) {
        AlertDialog(onDismissRequest = { showInfo = false }, title = { Text("ℹ️ О чате") }, text = {
            Column {
                val about = infoJson?.optString("about", "").orEmpty()
                if (about.isNotEmpty()) Text(about)
                val j = infoJson
                if (j == null) {
                    Text("загрузка…")
                } else {
                    listOf(
                        "Участников" to j.opt("participants_count"),
                        "Админов" to j.opt("admins_count"),
                        "Онлайн" to j.opt("online_count"),
                        "TTL" to j.opt("ttl_period"),
                    ).filter { it.second != null && it.second.toString() != "null" }
                        .forEach { (k, v) -> Text("$k: $v", style = MaterialTheme.typography.bodySmall) }
                    if (j.optBoolean("antispam", false)) Text("🛡 антиспам вкл", style = MaterialTheme.typography.bodySmall)
                    j.optLong("linked_chat_id", 0).takeIf { it != 0L }?.let { linked ->
                        TextButton(onClick = { showInfo = false; nav.navigate("chat/$linked") }) {
                            Text("→ Связанный чат $linked")
                        }
                    }
                }
            }
        }, confirmButton = { TextButton(onClick = { showInfo = false }) { Text("Закрыть") } })
    }
    if (showParticipants) {
        AlertDialog(onDismissRequest = { showParticipants = false }, title = { Text("👥 Участники") }, text = {
            LazyColumn(Modifier.heightIn(max = 400.dp)) {
                items(participants.size) { i ->
                    val pm = participants[i]
                    Text("• ${pm.optString("name", "?")}${if (pm.optBoolean("admin")) " 🛡" else ""}",
                        style = MaterialTheme.typography.bodySmall)
                }
            }
        }, confirmButton = { TextButton(onClick = { showParticipants = false }) { Text("Закрыть") } })
    }
    if (showScheduled) {
        AlertDialog(onDismissRequest = { showScheduled = false }, title = { Text("⏰ Запланированные") }, text = {
            LazyColumn(Modifier.heightIn(max = 400.dp)) {
                items(scheduled.size) { i ->
                    val m = scheduled[i]
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text("${m.text.take(80)}", modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodySmall)
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    Core.call("cancel_scheduled", mapOf("chat" to chatId, "id" to m.id))
                                    scheduled = scheduled.filter { it.id != m.id }
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("✕") }
                    }
                }
            }
        }, confirmButton = { TextButton(onClick = { showScheduled = false }) { Text("Закрыть") } })
    }
    if (showPinned) {
        AlertDialog(onDismissRequest = { showPinned = false }, title = { Text("📌 Закреплённые") }, text = {
            LazyColumn(Modifier.heightIn(max = 400.dp)) {
                items(pinned.size) { i ->
                    Text(pinned[i].text.take(120), style = MaterialTheme.typography.bodySmall)
                }
            }
        }, confirmButton = { TextButton(onClick = { showPinned = false }) { Text("Закрыть") } })
    }
    if (showPoll) {
        AlertDialog(onDismissRequest = { showPoll = false }, title = { Text("📊 Новый опрос") }, text = {
            Column {
                OutlinedTextField(pollQ, { pollQ = it }, label = { Text("Вопрос") })
                OutlinedTextField(pollOpts, { pollOpts = it }, label = { Text("Варианты через ;") })
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        val opts = pollOpts.split(";").map { it.trim() }.filter { it.isNotEmpty() }
                        Core.call("create_poll", mapOf("chat" to chatId, "question" to pollQ, "options" to opts, "quiz" to false))
                        pollQ = ""; pollOpts = ""; showPoll = false
                        reload()
                        status = "Опрос создан"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Создать") }
        }, dismissButton = { TextButton(onClick = { showPoll = false }) { Text("Отмена") } })
    }
    if (showTtl) {
        var secs by remember { mutableStateOf("86400") }
        AlertDialog(onDismissRequest = { showTtl = false }, title = { Text("⏳ Автоудаление") }, text = {
            OutlinedTextField(secs, { secs = it }, label = { Text("Секунды (0 — выкл)") })
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("set_ttl", mapOf("chat" to chatId, "secs" to (secs.toLongOrNull() ?: 0)))
                        showTtl = false
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Применить") }
        }, dismissButton = { TextButton(onClick = { showTtl = false }) { Text("Отмена") } })
    }
    if (showExport) {
        var fmt by remember { mutableStateOf("html") }
        var withMedia by remember { mutableStateOf(false) }
        AlertDialog(onDismissRequest = { showExport = false }, title = { Text("⤵ Экспорт") }, text = {
            Column {
                Row {
                    FilterChip(fmt == "html", { fmt = "html" }, { Text("HTML") })
                    Spacer(Modifier.width(8.dp))
                    FilterChip(fmt == "json", { fmt = "json" }, { Text("JSON") })
                }
                Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                    Checkbox(withMedia, { withMedia = it })
                    Text("Скачать медиа (до 150)")
                }
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        val path = File(ctx.getExternalFilesDir(null), "history_${chatId}.$fmt").absolutePath
                        val r = Core.call("export_history", mapOf("chat" to chatId, "format" to fmt, "with_media" to withMedia, "path" to path))
                        status = "Экспорт: ${r.optString("path")} (${r.optInt("count")} сообщений)"
                        showExport = false
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Экспортировать") }
        }, dismissButton = { TextButton(onClick = { showExport = false }) { Text("Отмена") } })
    }
}
