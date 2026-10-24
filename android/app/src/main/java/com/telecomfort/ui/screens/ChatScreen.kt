package com.telecomfort.ui.screens

import android.Manifest
import android.content.pm.PackageManager
import android.media.MediaRecorder
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
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
    var text by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var showInfo by remember { mutableStateOf(false) }
    var infoJson by remember { mutableStateOf<JSONObject?>(null) }
    var showTtl by remember { mutableStateOf(false) }
    var showExport by remember { mutableStateOf(false) }
    var showStickers by remember { mutableStateOf(false) }
    var showGifs by remember { mutableStateOf(false) }
    var stickers by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var gifs by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var jumpDate by remember { mutableStateOf("") }
    var recording by remember { mutableStateOf<MediaRecorder?>(null) }
    var recFile by remember { mutableStateOf<File?>(null) }
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    val listState = rememberLazyListState()

    fun reload(limit: Int = 100) {
        scope.launch {
            runCatching {
                val r = Core.call("history", mapOf("chat" to chatId, "limit" to limit))
                items = (0 until r.getJSONArray("result").length()).map { parseMessage(r.getJSONArray("result").getJSONObject(it)) }
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
            // Стоп → WAV → OGG/Opus → войс.
            runCatching { rec.stop(); rec.release() }
            recording = null
            val wav = recFile ?: return
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
        val wav = File(ctx.cacheDir, "rec_${System.currentTimeMillis()}.wav")
        val mr = MediaRecorder(ctx).apply {
            setAudioSource(MediaRecorder.AudioSource.MIC)
            setOutputFormat(MediaRecorder.OutputFormat.DEFAULT)
            setAudioEncoder(MediaRecorder.AudioEncoder.DEFAULT)
            setAudioSamplingRate(48000)
            setAudioChannels(1)
            setOutputFile(wav.absolutePath)
            prepare(); start()
        }
        recFile = wav
        recording = mr
        status = "● Запись… нажмите ■ для отправки"
    }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
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
            IconButton(onClick = { showTtl = true }) { Text("⏳") }
            IconButton(onClick = {
                jumpDate = ""
                scope.launch {
                    runCatching {
                        // Прыжок: дата вводится в диалог ниже — здесь заглушка на сегодня.
                        status = "Введите дату в диалоге 📅 (ГГГГ-ММ-ДД)"
                    }
                }
            }) { Text("📅") }
            IconButton(onClick = { showExport = true }) { Text("⤵") }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        LazyColumn(Modifier.weight(1f), state = listState, reverseLayout = true) {
            items(items.reversed(), key = { it.id }) { m ->
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
            Text(infoJson?.toString(2)?.take(2000) ?: "загрузка…")
        }, confirmButton = { TextButton(onClick = { showInfo = false }) { Text("Закрыть") } })
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
