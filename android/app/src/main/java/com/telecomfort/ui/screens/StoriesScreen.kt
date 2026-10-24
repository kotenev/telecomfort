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
fun StoriesScreen(nav: NavController, store: com.telecomfort.core.SecureStore? = null) {
    var items by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var scores by remember { mutableStateOf<Map<Int, JSONObject>>(emptyMap()) }
    var caption by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var showArchived by remember { mutableStateOf(false) }
    var cameraUri by remember { mutableStateOf<android.net.Uri?>(null) }
    var myId by remember { mutableStateOf<Long?>(null) }
    var editStory by remember { mutableStateOf<org.json.JSONObject?>(null) }
    var editCaption by remember { mutableStateOf("") }
    var deleteStory by remember { mutableStateOf<org.json.JSONObject?>(null) }
    var showAlbums by remember { mutableStateOf(false) }
    var albums by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var albumStories by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var musicPath by remember { mutableStateOf<String?>(null) }
    var viewersJson by remember { mutableStateOf<JSONObject?>(null) }
    var showViewers by remember { mutableStateOf(false) }
    var yandexToken by remember { mutableStateOf(store?.yandexToken ?: "") }
    var showPublish by remember { mutableStateOf(false) }
    var viewerPath by remember { mutableStateOf<String?>(null) }
    var viewerVideo by remember { mutableStateOf(false) }
    var viewerCaption by remember { mutableStateOf("") }
    var showInAppCamera by remember { mutableStateOf(false) }
    val camPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (ok) showInAppCamera = true else status = "Нет разрешения на камеру"
    }
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        scope.launch {
            runCatching { myId = Core.call("me", emptyMap()).optLong("id", 0).takeIf { it != 0L } }
        }
    }
    val ctx = LocalContext.current

    fun reload() {
        scope.launch {
            runCatching {
                val r = Core.call("stories")
                items = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                // Скоринг каждой сторис.
                val m = mutableMapOf<Int, JSONObject>()
                for (s in items) {
                    val cap = s.optString("caption", "")
                    // URL-детект на клиенте: ссылка в тексте/кнопке сторис.
                    val hasUrl = cap.contains("http", ignoreCase = true) || cap.contains("www.", ignoreCase = true) ||
                        cap.contains("t.me", ignoreCase = true) || s.optBoolean("has_url", false)
                    val sc = Core.call("ad_score", mapOf("caption" to cap, "has_url" to hasUrl, "has_qr" to false))
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
    fun publishStory(dst: File) {
        scope.launch {
            runCatching {
                // Эвристика: если в картинке найден QR — предупреждаем в статусе.
                val qr = runCatching {
                    Core.call("qr_hint", mapOf("path" to dst.absolutePath)).optBoolean("has_qr", false)
                }.getOrDefault(false)
                val params = mutableMapOf<String, Any>("path" to dst.absolutePath, "caption" to caption)
                musicPath?.let { params["music"] = it }
                val r = Core.call("story_publish", params)
                status = "Сторис опубликована (id ${r.optInt("id")})" + if (qr) " · ⚠ найден QR" else ""
                caption = ""
                musicPath = null
                showPublish = false
                reload()
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    fun handleNewMedia(uri: android.net.Uri) {
        scope.launch {
            runCatching {
                val dst = File(ctx.cacheDir, "story_${System.currentTimeMillis()}")
                ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                publishStory(dst)
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    val takePhoto = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { ok ->
        if (ok) cameraUri?.let { handleNewMedia(it) }
    }
    val takeVideo = rememberLauncherForActivityResult(ActivityResultContracts.CaptureVideo()) { ok ->
        if (ok) cameraUri?.let { handleNewMedia(it) }
    }
    fun launchCamera(video: Boolean) {
        val dst = File(ctx.cacheDir, "cam_${System.currentTimeMillis()}" + if (video) ".mp4" else ".jpg")
        val uri = androidx.core.content.FileProvider.getUriForFile(ctx, ctx.packageName + ".files", dst)
        cameraUri = uri
        if (video) takeVideo.launch(uri) else takePhoto.launch(uri)
    }
    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            runCatching {
                val dst = File(ctx.cacheDir, "story_${System.currentTimeMillis()}")
                ctx.contentResolver.openInputStream(uri)?.use { it.copyTo(dst.outputStream()) }
                publishStory(dst)
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
            Text("Сторис", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("enable_stealth", emptyMap())
                        status = "Стелс-режим включён (просмотры не засчитываются)"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("🥷") }
            TextButton(onClick = { showPublish = true }) { Text("＋ Опубликовать") }
            TextButton(onClick = {
                showAlbums = true
                scope.launch {
                    runCatching {
                        val r = Core.call("story_albums", emptyMap())
                        albums = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("🖼") }
            TextButton(onClick = {
                showArchived = !showArchived
                scope.launch {
                    runCatching {
                        val r = Core.call(if (showArchived) "archived_stories" else "stories", emptyMap())
                        items = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text(if (showArchived) "Лента" else "🗄 Архив") }
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        LazyColumn(Modifier.weight(1f)) {
            itemsIndexed(items, key = { i, s -> "s-$i-${s.optInt("id")}" }) { _, s ->
                val sc = scores[s.optInt("id")]
                val score = sc?.optDouble("score", 0.0) ?: 0.0
                Card(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp)) {
                    Column(Modifier.padding(8.dp)) {
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Text(s.optString("caption", "(без подписи)").take(120), modifier = Modifier.weight(1f))
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        val dl = Core.call("download_story_media", mapOf(
                                            "peer" to s.optLong("peer_id"), "story" to s.optInt("id")))
                                        val p = dl.optString("path")
                                        viewerVideo = p.endsWith(".mp4", true) || p.endsWith(".mkv", true) ||
                                            p.endsWith(".webm", true)
                                        viewerCaption = s.optString("caption", "")
                                        viewerPath = p
                                    }.onFailure { status = "Медиа недоступно: ${it.message}" }
                                }
                            }) { Text("👁") }
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
                                            Core.call("hide_peer_stories", mapOf("peer" to s.optLong("peer_id"), "hidden" to true))
                                            status = "Сторис автора скрыты"
                                            reload()
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("🚫 Скрыть") }
                                if (myId != null && s.optLong("peer_id") == myId) {
                                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                Core.call("story_react", mapOf("peer" to s.optLong("peer_id"), "story" to s.optInt("id"), "emoji" to "❤"))
                                status = "Реакция отправлена"
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text("❤️") }
                    var replyText by remember { mutableStateOf("") }
                    var showReply by remember { mutableStateOf(false) }
                    TextButton(onClick = { showReply = true }) { Text("💬") }
                    if (showReply) {
                        AlertDialog(onDismissRequest = { showReply = false }, title = { Text("Ответ на сторис") }, text = {
                            OutlinedTextField(replyText, { replyText = it }, label = { Text("Текст") })
                        }, confirmButton = {
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        Core.call("story_reply", mapOf("peer" to s.optLong("peer_id"), "story" to s.optInt("id"), "text" to replyText))
                                        replyText = ""
                                        showReply = false
                                        status = "Ответ отправлен"
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("Отправить") }
                        }, dismissButton = { TextButton(onClick = { showReply = false }) { Text("Отмена") } })
                    }
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                Core.call("toggle_story_pin", mapOf("story" to s.optInt("id"), "pinned" to true))
                                status = "Сторис закреплена в профиле"
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text("📌") }
                    TextButton(onClick = { editStory = s; editCaption = s.optString("caption", "") }) { Text("✏️") }
                                    TextButton(onClick = { deleteStory = s }) { Text("🗑") }
                                }
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
    if (showAlbums) {
        var newAlbum by remember { mutableStateOf("") }
        AlertDialog(onDismissRequest = { showAlbums = false }, title = { Text("🖼 Альбомы сторис") }, text = {
            Column {
                albums.forEach { a ->
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text("• ${a.optString("title", "?")} (${a.optInt("count", 0)})", modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodySmall)
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    val r = Core.call("album_stories", mapOf("album" to a.optInt("id")))
                                    albumStories = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("👁") }
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    Core.call("delete_album", mapOf("album" to a.optInt("id")))
                                    val r = Core.call("story_albums", emptyMap())
                                    albums = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("✕") }
                    }
                }
                if (albumStories.isNotEmpty()) {
                    Text("Сторис альбома:", style = MaterialTheme.typography.bodySmall)
                    albumStories.take(10).forEach { st ->
                        Text("– ${st.optString("caption", "?").take(60)}", style = MaterialTheme.typography.bodySmall)
                    }
                }
                OutlinedTextField(newAlbum, { newAlbum = it }, label = { Text("Название (из последних N сторис)") })
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        // Собираем альбом из последних 10 сторис ленты.
                        val ids = items.take(10).map { it.optInt("id") }
                        Core.call("create_album", mapOf("title" to newAlbum, "ids" to ids))
                        newAlbum = ""
                        val r = Core.call("story_albums", emptyMap())
                        albums = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Создать") }
        }, dismissButton = { TextButton(onClick = { showAlbums = false }) { Text("Закрыть") } })
    }
    if (editStory != null) {
        val st = editStory!!
        AlertDialog(onDismissRequest = { editStory = null }, title = { Text("✏️ Подпись сторис") }, text = {
            OutlinedTextField(editCaption, { editCaption = it }, label = { Text("Новая подпись") })
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("edit_story_caption", mapOf("story" to st.optInt("id"), "caption" to editCaption))
                        editStory = null
                        reload()
                        status = "Подпись обновлена"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Сохранить") }
        }, dismissButton = { TextButton(onClick = { editStory = null }) { Text("Отмена") } })
    }
    if (deleteStory != null) {
        val st = deleteStory!!
        AlertDialog(onDismissRequest = { deleteStory = null }, title = { Text("🗑 Удалить сторис?") }, text = {
            Text("Своя сторис #${st.optInt("id")} будет удалена безвозвратно.")
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("delete_stories", mapOf("ids" to listOf(st.optInt("id"))))
                        deleteStory = null
                        reload()
                        status = "Сторис удалена"
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Удалить", color = MaterialTheme.colorScheme.error) }
        }, dismissButton = { TextButton(onClick = { deleteStory = null }) { Text("Отмена") } })
    }
    if (showPublish) {
        AlertDialog(onDismissRequest = { showPublish = false }, title = { Text("Новая сторис") }, text = {
            Column {
                OutlinedTextField(caption, { caption = it }, label = { Text("Подпись") })
                Text("Файл и музыку (.mp3/.m4a) выберите кнопками ниже.",
                    style = MaterialTheme.typography.bodySmall)
            }
        }, confirmButton = {
            Row {
                TextButton(onClick = { pickMusic.launch("audio/*") }) { Text("🎵${if (musicPath != null) " ✓" else ""}") }
                TextButton(onClick = { pickMedia.launch("*/*") }) { Text("Файл") }
                TextButton(onClick = { launchCamera(false) }) { Text("📷") }
                TextButton(onClick = {
                    val granted = androidx.core.content.ContextCompat.checkSelfPermission(
                        ctx, android.Manifest.permission.CAMERA
                    ) == android.content.pm.PackageManager.PERMISSION_GRANTED
                    if (granted) showInAppCamera = true else camPerm.launch(android.Manifest.permission.CAMERA)
                }) { Text("📸 Превью") }
                TextButton(onClick = { launchCamera(true) }) { Text("🎬") }
            }
        }, dismissButton = { TextButton(onClick = { showPublish = false }) { Text("Отмена") } })
    }
    if (showInAppCamera) {
        InAppCamera(
            onCapture = { f ->
                showInAppCamera = false
                publishStory(f)
            },
            onClose = { showInAppCamera = false },
        )
    }
    viewerPath?.let { path ->
        androidx.compose.ui.window.Dialog(
            onDismissRequest = { viewerPath = null },
            properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Surface(color = androidx.compose.ui.graphics.Color.Black, modifier = Modifier.fillMaxSize()) {
                Column(Modifier.fillMaxSize().padding(8.dp)) {
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text(viewerCaption.take(60), color = androidx.compose.ui.graphics.Color.White,
                            modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
                        TextButton(onClick = { viewerPath = null }) { Text("✕", color = androidx.compose.ui.graphics.Color.White) }
                    }
                    if (viewerVideo) {
                        androidx.compose.ui.viewinterop.AndroidView(
                            factory = { c ->
                                android.widget.VideoView(c).apply {
                                    setVideoPath(path)
                                    setMediaController(android.widget.MediaController(c).also { it.setAnchorView(this) })
                                    setOnPreparedListener { it.isLooping = true; start() }
                                }
                            },
                            modifier = Modifier.fillMaxSize(),
                        )
                    } else {
                        val bmp = remember(path) { android.graphics.BitmapFactory.decodeFile(path) }
                        if (bmp != null) {
                            androidx.compose.foundation.Image(
                                bmp.asImageBitmap(), contentDescription = null,
                                modifier = Modifier.fillMaxSize(),
                                contentScale = androidx.compose.ui.layout.ContentScale.Fit,
                            )
                        } else {
                            Text("Не удалось открыть сторис", color = androidx.compose.ui.graphics.Color.White)
                        }
                    }
                }
            }
        }
    }
}
