package com.telecomfort.ui.screens

import android.graphics.BitmapFactory
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.telecomfort.core.Core
import com.telecomfort.core.parseStory
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

/**
 * Сторис: детект рекламы (бейдж + причины), блюр, сохранение в Яндекс.Диск,
 * публикация с музыкой, архив.
 */
@Composable
fun StoriesScreen(nav: NavController) {
    var items by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var scores by remember { mutableStateOf<Map<Int, JSONObject>>(emptyMap()) }
    var caption by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var musicPath by remember { mutableStateOf<String?>(null) }
    var viewersJson by remember { mutableStateOf<JSONObject?>(null) }
    var showViewers by remember { mutableStateOf(false) }
    var yandexToken by remember { mutableStateOf("") }
    var showPublish by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current

    fun reload() {
        scope.launch {
            runCatching {
                val r = Core.call("stories")
                items = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                // Скоринг каждой сторис.
                val m = mutableMapOf<Int, JSONObject>()
                for (s in items) {
                    val sc = Core.call("ad_score", mapOf(
                        "caption" to s.optString("caption", ""),
                        "has_url" to s.optBoolean("has_url", false),
                        "has_qr" to false,
                    ))
                    m[s.optInt("id")] = sc
                }
                scores = m
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(Unit) { reload() }

    val pickMusic = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        val dst = File(ctx.cacheDir, "story_music_${System.currentTimeMillis()}.mp3")
        ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
        musicPath = dst.absolutePath
        status = "🎵 Музыка: ${dst.name}"
    }
    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            runCatching {
                val dst = File(ctx.cacheDir, "story_${System.currentTimeMillis()}")
                ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                val params = mutableMapOf<String, Any>("path" to dst.absolutePath, "caption" to caption)
                musicPath?.let { params["music"] = it }
                val r = Core.call("story_publish", params)
                status = "Сторис опубликована (id ${r.optInt("id")})"
                caption = ""
                musicPath = null
                showPublish = false
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
            Text("Сторис", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            TextButton(onClick = { showPublish = true }) { Text("＋ Опубликовать") }
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        LazyColumn(Modifier.weight(1f)) {
            itemsIndexed(items, key = { i, s -> "s-$i-${s.optInt("id")}" }) { _, s ->
                val sc = scores[s.optInt("id")]
                val score = sc?.optDouble("score", 0.0) ?: 0.0
                Card(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp)) {
                    Column(Modifier.padding(8.dp)) {
                        Row {
                            Text(s.optString("caption", "(без подписи)").take(120), modifier = Modifier.weight(1f))
                            if (score > 0.5) {
                                AssistChip(onClick = {}, label = { Text("реклама ${"%.0f".format(score * 100)}%") })
                            }
                        }
                        if (score > 0.5) {
                            Text(
                                (0 until (sc?.getJSONArray("reasons")?.length() ?: 0))
                                    .map { sc!!.getJSONArray("reasons").getString(it) }
                                    .joinToString("; "),
                                style = MaterialTheme.typography.bodySmall,
                            )
                            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            val dl = Core.call("download_story_media", mapOf(
                                                "peer" to s.optLong("peer_id"), "story" to s.optInt("id")))
                                            val src = dl.optString("path")
                                            val dst = "$src.blur.jpg"
                                            Core.call("blur_full", mapOf("src" to src, "dst" to dst, "sigma" to 12.0))
                                            status = "Блюр сохранён: $dst"
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("🫣 Блюр") }
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            if (yandexToken.isBlank()) { status = "Введите Yandex OAuth-токен в настройках"; return@launch }
                                            val dl = Core.call("download_story_media", mapOf(
                                                "peer" to s.optLong("peer_id"), "story" to s.optInt("id")))
                                            val src = dl.optString("path")
                                            Core.call("yandex_upload", mapOf(
                                                "token" to yandexToken,
                                                "remote_folder" to "telecomfort/stories",
                                                "local" to src,
                                                "remote" to "telecomfort/stories/${File(src).name}",
                                            ))
                                            status = "Загружено в Яндекс.Диск"
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("☁️ в Яндекс") }
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            Core.call("forward_story", mapOf("peer" to s.optLong("peer_id"), "story" to s.optInt("id")))
                                            status = "Репост опубликован"
                                            reload()
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("↗️ Репост") }
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            val r = Core.call("story_viewers", mapOf("story" to s.optInt("id"), "limit" to 100))
                                            viewersJson = r
                                            showViewers = true
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("👁 ${sc?.optInt("views", 0) ?: ""}") }
                            }
                        }
                    }
                }
            }
        }
        if (status.isNotEmpty()) Text(status, modifier = Modifier.padding(8.dp), style = MaterialTheme.typography.bodySmall)
    }
    if (showViewers) {
        AlertDialog(onDismissRequest = { showViewers = false }, title = { Text("👁 Просмотры") }, text = {
            Column {
                Text("Всего: ${viewersJson?.optInt("count", 0)}")
                (0 until (viewersJson?.optJSONArray("viewers")?.length() ?: 0)).map {
                    viewersJson!!.getJSONArray("viewers").getJSONObject(it)
                }.take(50).forEach { vw ->
                    Text("• ${vw.optString("name", "?")}", style = MaterialTheme.typography.bodySmall)
                }
            }
        }, confirmButton = { TextButton(onClick = { showViewers = false }) { Text("Закрыть") } })
    }
    if (showPublish) {
        AlertDialog(onDismissRequest = { showPublish = false }, title = { Text("Новая сторис") }, text = {
            Column {
                OutlinedTextField(caption, { caption = it }, label = { Text("Подпись") })
                Text("Файл выберете на следующем шаге; музыка — из файла .mp3/.m4a (TODO: второй picker).",
                    style = MaterialTheme.typography.bodySmall)
            }
        }, confirmButton = {
            Row {
                TextButton(onClick = { pickMusic.launch("audio/*") }) { Text("🎵 Музыка${if (musicPath != null) " ✓" else ""}") }
                TextButton(onClick = { pickMedia.launch("*/*") }) { Text("Выбрать файл") }
            }
        }, dismissButton = { TextButton(onClick = { showPublish = false }) { Text("Отмена") } })
    }
}
