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
    var replyTo by remember { mutableStateOf<Pair<Int, String>?>(null) }
    var editMsg by remember { mutableStateOf<Message?>(null) }
    var forwardIds by remember { mutableStateOf<List<Int>>(emptyList()) }
    var forwardTo by remember { mutableStateOf<Long?>(null) }
    var forwardChats by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var text by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var showInfo by remember { mutableStateOf(false) }
    var showParticipants by remember { mutableStateOf(false) }
    var statsJson by remember { mutableStateOf<org.json.JSONObject?>(null) }
    var graphGrowth by remember { mutableStateOf<Pair<List<String>, List<Double>>?>(null) }
    var graphInter by remember { mutableStateOf<Pair<List<String>, List<Double>>?>(null) }
    var showGraphs by remember { mutableStateOf(false) }
    var participants by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var showScheduled by remember { mutableStateOf(false) }
    var showTopics by remember { mutableStateOf(false) }
    var topics by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var topicId by remember { mutableStateOf<Int?>(null) }
    var votersTitle by remember { mutableStateOf<String?>(null) }
    var recording by remember { mutableStateOf<com.telecomfort.core.PcmRecorder?>(null) }
    var recLevel by remember { mutableStateOf(0f) }
    LaunchedEffect(recording) {
        while (recording != null) {
            recLevel = recording?.level ?: 0f
            kotlinx.coroutines.delay(120)
        }
    }
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var showShareContact by remember { mutableStateOf(false) }
    var shareName by remember { mutableStateOf("") }
    var sharePhone by remember { mutableStateOf("") }
    val pickContact = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.PickContact()
    ) { uri ->
        if (uri != null) {
            try {
                ctx.contentResolver.query(uri, null, null, null, null)?.use { c ->
                    if (c.moveToFirst()) {
                        val nameIdx = c.getColumnIndex(android.provider.ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME)
                        val phoneIdx = c.getColumnIndex(android.provider.ContactsContract.CommonDataKinds.Phone.NUMBER)
                        if (nameIdx >= 0) shareName = c.getString(nameIdx) ?: ""
                        if (phoneIdx >= 0) sharePhone = c.getString(phoneIdx) ?: ""
                    }
                }
                showShareContact = true
            } catch (e: Exception) { status = "Контакт не прочитан: ${e.message}" }
        }
    }
    var voters by remember { mutableStateOf<List<org.json.JSONObject>>(emptyList()) }
    var scheduled by remember { mutableStateOf<List<Message>>(emptyList()) }
    var showPinned by remember { mutableStateOf(false) }
    var pinned by remember { mutableStateOf<List<Message>>(emptyList()) }
    var showPoll by remember { mutableStateOf(false) }
    var showSchedule by remember { mutableStateOf(false) }
    var schedDate by remember { mutableStateOf("") }
    var schedTime by remember { mutableStateOf("") }
    var pollQ by remember { mutableStateOf("") }
    var pollOpts by remember { mutableStateOf("") }
    var pollQuiz by remember { mutableStateOf(false) }
    var pollCorrect by remember { mutableStateOf("0") }
    var infoJson by remember { mutableStateOf<JSONObject?>(null) }
    var showTtl by remember { mutableStateOf(false) }
    var showExport by remember { mutableStateOf(false) }
    var showStickers by remember { mutableStateOf(false) }
    var showGifs by remember { mutableStateOf(false) }
    var stickers by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var gifs by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var jumpDate by remember { mutableStateOf("") }
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
    LaunchedEffect(chatId) {
        reload()
        scope.launch {
            // Помечаем прочитанным при открытии (как десктопный select_chat).
            runCatching { Core.call("mark_read", mapOf("chat" to chatId)) }
        }
    }

    val pickRound = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            runCatching {
                val dst = File(ctx.cacheDir, "round_${System.currentTimeMillis()}.mp4")
                ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                val r = Core.call("send_round", mapOf("chat" to chatId, "path" to dst.absolutePath, "caption" to text))
                status = "Кружок отправлен (msg ${r.optInt("id")})"
                text = ""
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        scope.launch {
            runCatching {
                val paths = uris.mapIndexed { i, uri ->
                    val dst = File(ctx.cacheDir, "attach_${System.currentTimeMillis()}_$i")
                    ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                    dst.absolutePath
                }
                if (paths.size == 1) {
                    val r = Core.call("send_file", mapOf("chat" to chatId, "path" to paths[0], "caption" to text))
                    status = "Отправлено (msg ${r.optInt("id")})"
                } else {
                    val r = Core.call("send_album", mapOf("chat" to chatId, "paths" to paths, "caption" to text))
                    status = "Альбом отправлен"
                }
                text = ""
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
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
                if (editMsg != null) {
                    Core.call("edit_text", mapOf("chat" to chatId, "id" to editMsg!!.id, "text" to text))
                    editMsg = null
                } else if (topicId != null) {
                    Core.call("send_to_topic", mapOf("chat" to chatId, "topic" to topicId!!, "text" to text))
                    } else {
                        val params = mutableMapOf<String, Any>("chat" to chatId, "text" to text)
                        replyTo?.let { params["reply_to"] = it.first }
                        Core.call("send_text", params)
                        replyTo = null
                    }
                text = ""
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }

    fun showPollVoters(title: String, msg: Int) {
        scope.launch {
            runCatching {
                val r = Core.call("poll_voters", mapOf("chat" to chatId, "id" to msg))
                votersTitle = title + " (" + r.optInt("count") + ")"
                val arr = r.optJSONArray("names") ?: org.json.JSONArray()
                voters = (0 until arr.length()).map { i ->
                    org.json.JSONObject().put("name", arr.getString(i))
                }
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
                        val r = Core.call("forum_topics", mapOf("chat" to chatId))
                        topics = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                        showTopics = true
                    }.onFailure { status = "Форум не найден (не супергруппа?)" }
                }
            }) { Text("🗂") }
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
            IconButton(onClick = { nav.navigate("stars") }) { Text("⭐") }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        var mediaFilter by remember { mutableStateOf("") }
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            listOf("" to "все", "photo" to "📷", "video" to "🎬", "links" to "🔗").forEach { (kind, label) ->
                FilterChip(selected = kind == mediaFilter, onClick = {
                    mediaFilter = kind
                    scope.launch {
                        runCatching {
                            if (kind.isEmpty()) reload()
                            else {
                                val r = Core.call("search_media", mapOf("chat" to chatId, "kind" to kind, "limit" to 100))
                                rawItems = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                items = rawItems.map { parseMessage(it) }
                            }
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }, label = { Text(label) })
            }
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
                    TextButton(onClick = {
                        val cm = ctx.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                        cm.setPrimaryClip(android.content.ClipData.newPlainText("telecomfort", m.text))
                        status = "Текст скопирован"
                    }) { Text("📋") }
                        TextButton(onClick = {
                            replyTo = m.id to m.text.take(80)
                            editMsg = null
                        }) { Text("↩") }
                        if (m.outgoing) {
                            TextButton(onClick = {
                                editMsg = m; text = m.text; replyTo = null
                            }) { Text("✏️") }
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        Core.call("delete_msg", mapOf("chat" to chatId, "id" to m.id))
                                        reload()
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("🗑") }
                        }
                        TextButton(onClick = {
                            forwardIds = listOf(m.id)
                            scope.launch {
                                runCatching {
                                    val r = Core.call("dialogs")
                                    forwardChats = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                    forwardTo = null
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("↗") }
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    Core.call("toggle_pin", mapOf("chat" to chatId, "id" to m.id))
                                    status = "Закреплено (повторный тап по закреплённому — открепить в окне 📌)"
                                    reload()
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("📌") }
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
                                if (m.outgoing && !poll.optBoolean("closed")) {
                                    TextButton(onClick = {
                                        scope.launch {
                                            runCatching {
                                                Core.call("stop_poll", mapOf("chat" to chatId, "id" to m.id))
                                                reload()
                                            }.onFailure { status = "Ошибка: ${it.message}" }
                                        }
                                    }) { Text("■ Закрыть опрос") }
                        TextButton(onClick = { showPollVoters("Опрос: " + (poll?.optString("question", "") ?: ""), m.id) }) { Text("👥 Кто") }
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
        replyTo?.let { (id, snippet) ->
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Text("↩ Ответ на #$id: ${snippet.take(60)}", modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodySmall)
                TextButton(onClick = { replyTo = null }) { Text("✕") }
            }
        }
        editMsg?.let { em ->
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Text("✏️ Правка #${em.id}", modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodySmall)
                TextButton(onClick = { editMsg = null; text = "" }) { Text("✕") }
            }
        }
        topicId?.let { tid ->
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            Text("🗂 Топик #$tid — отправка идёт туда", modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodySmall)
            TextButton(onClick = { topicId = null; reload() }) { Text("✕") }
        }
    }
    Row(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            OutlinedTextField(text, { text = it }, modifier = Modifier.weight(1f), placeholder = { Text(if (editMsg != null) "Новый текст…" else "Сообщение…") })
            IconButton(onClick = { sendText() }) { Icon(Icons.Default.Send, "Отправить") }
            IconButton(onClick = { pickFiles.launch("*/*") }) { Text("📎") }
            IconButton(onClick = { pickContact.launch(null) }) { Text("👤") }
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
                        val loc = try {
                            val client = com.google.android.gms.location.LocationServices
                                .getFusedLocationProviderClient(ctx)
                            var res: android.location.Location? = null
                            val task = client.lastLocation
                            var done = false
                            task.addOnSuccessListener { res = it; done = true }
                            task.addOnFailureListener { done = true }
                            val t0 = System.currentTimeMillis()
                            while (!done && System.currentTimeMillis() - t0 < 3000) kotlinx.coroutines.delay(100)
                            res
                        } catch (_: SecurityException) { null }
                        val lat = loc?.latitude ?: 55.75
                        val lon = loc?.longitude ?: 37.61
                        Core.call("send_geo", mapOf("chat" to chatId, "lat" to lat, "long" to lon))
                        status = "📍 ${"%.4f".format(lat)}, ${"%.4f".format(lon)}"
                        reload()
                    }.onFailure { status = "Гео недоступно (нужно разрешение): ${it.message}" }
                }
            }) { Text("📍") }
            IconButton(onClick = { showSchedule = true }) { Text("⏰") }
            if (recording != null) {
            val bars = "▁▂▃▄▅▆▇".take(((recLevel * 7).toInt()).coerceIn(1, 7))
            Text("● $bars", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }
        IconButton(onClick = { toggleRecord() }) { Text(if (recording != null) "■" else "🎙") }
            IconButton(onClick = { pickRound.launch("video/*") }) { Text("◉") }
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

    var showClear by remember { mutableStateOf(false) }
    var isBlocked by remember { mutableStateOf(false) }
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
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                statsJson = Core.call("channel_stats", mapOf("chat" to chatId))
                                val g = Core.call("channel_graph", mapOf("chat" to chatId, "kind" to "growth"))
                                graphGrowth = parseSeries(g)
                                val gi = Core.call("channel_graph", mapOf("chat" to chatId, "kind" to "interactions"))
                                graphInter = parseSeries(gi)
                                showGraphs = true
                            }.onFailure { status = "Статистика недоступна: ${it.message}" }
                        }
                    }) { Text("📊 Статистика и графики") }
                    if (showGraphs) {
                        statsJson?.let { st ->
                            Text("Подписчики: ${st.optInt("followers")} · Просмотры/пост: ${"%.0f".format(st.optDouble("views_per_post"))}", style = MaterialTheme.typography.bodySmall)
                        }
                        Text("📈 Рост", style = MaterialTheme.typography.bodySmall)
                        graphGrowth?.let { (l, v) -> MiniChart(l, v) }
                        Text("💬 Вовлечённость", style = MaterialTheme.typography.bodySmall)
                        graphInter?.let { (l, v) -> MiniChart(l, v) }
                        TextButton(onClick = { showGraphs = false }) { Text("Скрыть графики") }
                    }
                    var peerInfo by remember { mutableStateOf<org.json.JSONObject?>(null) }
                    var invite by remember { mutableStateOf<String?>(null) }
                    var slowmode by remember { mutableStateOf("0") }
                    LaunchedEffect(chatId) {
                        scope.launch {
                            runCatching { peerInfo = Core.call("peer_full", mapOf("chat" to chatId)) }.onFailure {}
                            runCatching { invite = Core.call("invite_link", mapOf("chat" to chatId)).optString("link") }.onFailure {}
                        }
                    }
                    peerInfo?.let { pf ->
                        val about = pf.optString("about", "")
                        if (about.isNotEmpty()) Text("О себе: $about", style = MaterialTheme.typography.bodySmall)
                        val flags = listOf("⭐ Premium" to pf.optBoolean("premium"), "✓ верифицирован" to pf.optBoolean("verified"), "⚠ SCAM" to pf.optBoolean("scam"), "⛔ фейк" to pf.optBoolean("fake"))
                            .filter { it.second }.joinToString(" · ") { it.first }
                        if (flags.isNotEmpty()) Text(flags, style = MaterialTheme.typography.bodySmall)
                    }
                    invite?.let { link ->
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Text("🔗 $link", modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
                            TextButton(onClick = {
                                val cm = ctx.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                                cm.setPrimaryClip(android.content.ClipData.newPlainText("invite", link))
                                status = "Ссылка скопирована"
                            }) { Text("📋") }
                        }
                    }
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        OutlinedTextField(slowmode, { slowmode = it }, label = { Text("Слоумод, сек") }, modifier = Modifier.weight(1f))
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    Core.call("set_slowmode", mapOf("chat" to chatId, "secs" to (slowmode.toLongOrNull() ?: 0)))
                                    status = "Слоумод обновлён"
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("ОК") }
                    }
                    var antispamOn by remember { mutableStateOf(j.optBoolean("antispam", false)) }
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Checkbox(antispamOn, { v ->
                            antispamOn = v
                            scope.launch {
                                runCatching {
                                    Core.call("toggle_antispam", mapOf("chat" to chatId, "enabled" to v))
                                    status = "Антиспам: ${if (v) "вкл" else "выкл"}"
                                }.onFailure { status = "Ошибка (нужны права?): ${it.message}" }
                            }
                        })
                        Text("Антиспам (админы)", style = MaterialTheme.typography.bodySmall)
                    }
                    TextButton(onClick = { showClear = true }) { Text("🧹 Очистить историю", color = MaterialTheme.colorScheme.error) }
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                Core.call("block_user", mapOf("chat" to chatId, "blocked" to !isBlocked))
                                isBlocked = !isBlocked
                                status = if (isBlocked) "Заблокирован" else "Разблокирован"
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text(if (isBlocked) "✓ Разблокировать" else "🚫 Заблокировать") }
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                Core.call("leave_channel", mapOf("chat" to chatId))
                                nav.popBackStack()
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }

                    }) { Text("🚪 Покинуть", color = MaterialTheme.colorScheme.error) }
                    j.optLong("linked_chat_id", 0).takeIf { it != 0L }?.let { linked ->
                        TextButton(onClick = { showInfo = false; nav.navigate("chat/$linked") }) {
                            Text("→ Связанный чат $linked")
                        }
                    }
                }
            }
        }, confirmButton = { TextButton(onClick = { showInfo = false }) { Text("Закрыть") } })
    }
    if (showClear) {
        AlertDialog(onDismissRequest = { showClear = false }, title = { Text("Очистить историю?") }, text = {
            Column { Text("Удаление у всех (revoke). Действие необратимо.") }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("clear_history", mapOf("chat" to chatId, "revoke" to true))
                        showClear = false
                        showInfo = false
                        reload()
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Очистить", color = MaterialTheme.colorScheme.error) }
        }, dismissButton = { TextButton(onClick = { showClear = false }) { Text("Отмена") } })
    }
    if (showParticipants) {
        AlertDialog(onDismissRequest = { showParticipants = false }, title = { Text("👥 Участники") }, text = {
            LazyColumn(Modifier.heightIn(max = 400.dp)) {
                items(participants.size) { i ->
                    val pm = participants[i]
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text("• ${pm.optString("name", "?")}${if (pm.optBoolean("admin")) " 🛡" else ""}",
                            modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
                        if (!pm.optBoolean("admin")) {
                            var showAdmin by remember { mutableStateOf(false) }
                        var aTitle by remember { mutableStateOf("") }
                        var aChangeInfo by remember { mutableStateOf(true) }
                        var aPost by remember { mutableStateOf(false) }
                        var aDelete by remember { mutableStateOf(false) }
                        var aBan by remember { mutableStateOf(false) }
                        var aInvite by remember { mutableStateOf(true) }
                        var aPin by remember { mutableStateOf(false) }
                        TextButton(onClick = { showAdmin = true }) { Text("🛡") }
                        if (showAdmin) {
                            AlertDialog(onDismissRequest = { showAdmin = false }, title = { Text("Права: ${pm.optString("name")}") }, text = {
                                Column {
                                    OutlinedTextField(aTitle, { aTitle = it }, label = { Text("Титул") })
                                    listOf("Изменять инфо" to aChangeInfo, "Постить" to aPost, "Удалять" to aDelete,
                                        "Банить" to aBan, "Приглашать" to aInvite, "Закреплять" to aPin).forEachIndexed { bi, (label, cur) ->
                                        var checked by remember { mutableStateOf(cur) }
                                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                                            Checkbox(checked, {
                                                checked = it
                                                when (bi) {
                                                    0 -> aChangeInfo = it
                                                    1 -> aPost = it
                                                    2 -> aDelete = it
                                                    3 -> aBan = it
                                                    4 -> aInvite = it
                                                    else -> aPin = it
                                                }
                                            })
                                            Text(label, style = MaterialTheme.typography.bodySmall)
                                        }
                                    }
                                }
                            }, confirmButton = {
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            Core.call("set_admin", mapOf(
                                                "chat" to chatId, "user" to pm.optLong("id"),
                                                "hash" to pm.optLong("access_hash"), "title" to aTitle,
                                                "rights" to mapOf(
                                                    "change_info" to aChangeInfo, "post_messages" to aPost,
                                                    "edit_messages" to aPost, "delete_messages" to aDelete,
                                                    "ban_users" to aBan, "invite_users" to aInvite,
                                                    "pin_messages" to aPin, "manage_call" to false,
                                                    "manage_topics" to false, "post_stories" to false,
                                                    "edit_stories" to false, "delete_stories" to false,
                                                    "manage_direct_messages" to false)))
                                            showAdmin = false
                                            status = "Админ назначен"
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("Назначить") }
                            }, dismissButton = { TextButton(onClick = { showAdmin = false }) { Text("Отмена") } })
                        }
                        TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        Core.call("kick_participant", mapOf("chat" to chatId, "user" to pm.optLong("id")))
                                        val r = Core.call("participants", mapOf("chat" to chatId, "limit" to 200))
                                        participants = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                    }.onFailure { status = "Ошибка (нужны права?): ${it.message}" }
                                }
                            }) { Text("✕") }
                        }
                    }
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
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text(pinned[i].text.take(120), modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodySmall)
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    Core.call("unpin_message", mapOf("chat" to chatId, "id" to pinned[i].id))
                                    val r = Core.call("pinned_list", mapOf("chat" to chatId, "limit" to 50))
                                    pinned = (0 until r.getJSONArray("result").length()).map { parseMessage(r.getJSONArray("result").getJSONObject(it)) }
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("✕") }
                    }
                }
            }
        }, confirmButton = { TextButton(onClick = { showPinned = false }) { Text("Закрыть") } })
    }
    if (showPoll) {
        AlertDialog(onDismissRequest = { showPoll = false }, title = { Text("📊 Новый опрос") }, text = {
            Column {
                OutlinedTextField(pollQ, { pollQ = it }, label = { Text("Вопрос") })
                OutlinedTextField(pollOpts, { pollOpts = it }, label = { Text("Варианты через ;") })
                Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                    Checkbox(pollQuiz, { pollQuiz = it })
                    Text("Викторина", style = MaterialTheme.typography.bodySmall)
                    OutlinedTextField(pollCorrect, { pollCorrect = it }, label = { Text("Правильный № (с 0)") }, modifier = Modifier.weight(1f))
                }
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        val opts = pollOpts.split(";").map { it.trim() }.filter { it.isNotEmpty() }
                        val correct = if (pollQuiz) pollCorrect.toIntOrNull() else null
                        buildMap {
                            put("chat", chatId); put("question", pollQ); put("options", opts); put("quiz", pollQuiz)
                            if (correct != null) put("correct", correct)
                        }.let { Core.call("create_poll", it) }
                        pollQ = ""; pollOpts = ""; pollCorrect = "0"; pollQuiz = false; showPoll = false
                        reload()
                        status = "Опрос создан"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Создать") }
        }, dismissButton = { TextButton(onClick = { showPoll = false }) { Text("Отмена") } })
    }
    if (forwardIds.isNotEmpty()) {
        var q by remember { mutableStateOf("") }
        AlertDialog(onDismissRequest = { forwardIds = emptyList() }, title = { Text("↗ Переслать в…") }, text = {
            Column {
                OutlinedTextField(q, { q = it }, label = { Text("Поиск чата") })
                LazyColumn(Modifier.heightIn(max = 300.dp)) {
                    val list = forwardChats.filter { q.isEmpty() || it.optString("title").contains(q, true) }
                    items(list.size) { i ->
                        val c = list[i]
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    val r = Core.call("forward", mapOf("from" to chatId, "ids" to forwardIds, "to" to c.optLong("id")))
                                    status = "Переслано: ${r.optInt("count")}"
                                    forwardIds = emptyList()
                                    reload()
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("→ ${c.optString("title")}") }
                    }
                }
            }
        }, confirmButton = {}, dismissButton = { TextButton(onClick = { forwardIds = emptyList() }) { Text("Отмена") } })
    }
    if (showSchedule) {
        AlertDialog(onDismissRequest = { showSchedule = false }, title = { Text("⏰ Отложенная отправка") }, text = {
            Column {
                Text("Текст возьмётся из композера.", style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(schedDate, { schedDate = it }, label = { Text("Дата ГГГГ-ММ-ДД") })
                OutlinedTextField(schedTime, { schedTime = it }, label = { Text("Время ЧЧ:ММ (МСК)") })
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        val dp = schedDate.trim().split("-").map { it.toInt() }
                        val tp = schedTime.trim().split(":").map { it.toInt() }
                        val cal = java.util.Calendar.getInstance(java.util.TimeZone.getTimeZone("Europe/Moscow"))
                        cal.set(dp[0], dp[1] - 1, dp[2], tp[0], tp[1], 0)
                        Core.call("send_scheduled", mapOf("chat" to chatId, "text" to text, "at" to cal.timeInMillis / 1000))
                        text = ""
                        showSchedule = false
                        status = "Запланировано"
                    }.onFailure { status = "Дата/время — ГГГГ-ММ-ДД и ЧЧ:ММ" }
                }
            }) { Text("Запланировать") }
        }, dismissButton = { TextButton(onClick = { showSchedule = false }) { Text("Отмена") } })
    }
    if (showTopics) {
        AlertDialog(onDismissRequest = { showTopics = false }, title = { Text("🗂 Топики") }, text = {
            LazyColumn(Modifier.heightIn(max = 400.dp)) {
                item {
                    TextButton(onClick = {
                        topicId = null
                        showTopics = false
                        reload()
                    }) { Text("← Вся история") }
                }
                items(topics.size) { i ->
                    val t = topics[i]
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                val r = Core.call("topic_history", mapOf("chat" to chatId, "topic" to t.optInt("id"), "limit" to 100))
                                topicId = t.optInt("id")
                                rawItems = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                items = rawItems.map { parseMessage(it) }
                                showTopics = false
                                status = "Топик: ${t.optString("title", "?")}"
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text("• ${t.optString("title", "?")}") }
                }
            }
        }, confirmButton = { TextButton(onClick = { showTopics = false }) { Text("Закрыть") } })
    }
    votersTitle?.let { title ->
        AlertDialog(onDismissRequest = { votersTitle = null }, title = { Text("👥 $title") }, text = {
            LazyColumn(Modifier.heightIn(max = 300.dp)) {
                items(voters.size) { i ->
                    Text("• ${voters[i].optString("name", "?")}", style = MaterialTheme.typography.bodySmall)
                }
            }
        }, confirmButton = { TextButton(onClick = { votersTitle = null }) { Text("Закрыть") } })
    }
    if (showShareContact) {
        AlertDialog(onDismissRequest = { showShareContact = false }, title = { Text("👤 Поделиться контактом") }, text = {
            Column {
                OutlinedTextField(shareName, { shareName = it }, label = { Text("Имя") })
                OutlinedTextField(sharePhone, { sharePhone = it }, label = { Text("Телефон") })
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("share_contact", mapOf("chat" to chatId, "phone" to sharePhone, "first" to shareName, "last" to ""))
                        showShareContact = false
                        reload()
                        status = "Контакт отправлен"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Отправить") }
        }, dismissButton = { TextButton(onClick = { showShareContact = false }) { Text("Отмена") } })
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
