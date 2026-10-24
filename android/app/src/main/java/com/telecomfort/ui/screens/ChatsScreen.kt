package com.telecomfort.ui.screens

import androidx.compose.foundation.clickable
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
import com.telecomfort.core.Chat
import com.telecomfort.core.Core
import com.telecomfort.core.parseChat
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Диалоги: поиск, закреп/мьют/архив, 📍 рядом, переход в чат/сторис/настройки. */
@Composable
fun ChatsScreen(nav: NavController) {
    var chats by remember { mutableStateOf<List<Chat>>(emptyList()) }
    var query by remember { mutableStateOf("") }
    var lat by remember { mutableStateOf("55.75") }
    var long by remember { mutableStateOf("37.61") }
    var nearby by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var showNearby by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("") }
    var gsearch by remember { mutableStateOf<List<Pair<Long, org.json.JSONObject?>>>(emptyList()) }
    var showSearch by remember { mutableStateOf(false) }
    var showFolders by remember { mutableStateOf(false) }
    var renameTarget by remember { mutableStateOf<org.json.JSONObject?>(null) }
    var renameTitle by remember { mutableStateOf("") }
    var folders by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var newFolder by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()

    fun reload() {
        scope.launch {
            runCatching {
                chats = Core.call("dialogs").getJSONArray("result").let { a ->
                    (0 until a.length()).map { parseChat(a.getJSONObject(it)) }
                }
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(Unit) { reload() }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            IconButton(onClick = { nav.navigate("stories") }) { Icon(Icons.Default.Star, "Сторис") }
            IconButton(onClick = { nav.navigate("stars") }) { Text("⭐") }
            IconButton(onClick = { nav.navigate("settings") }) { Icon(Icons.Default.Settings, "Настройки") }
            IconButton(onClick = { nav.navigate("contacts") }) { Icon(Icons.Default.Person, "Контакты") }
            var showCreate by remember { mutableStateOf(false) }
            IconButton(onClick = { showCreate = true }) { Text("＋") }
            if (showCreate) {
                var title by remember { mutableStateOf("") }
                var isChannel by remember { mutableStateOf(true) }
                AlertDialog(onDismissRequest = { showCreate = false }, title = { Text("＋ Создать") }, text = {
                    Column {
                        OutlinedTextField(title, { title = it }, label = { Text("Название") })
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Checkbox(isChannel, { isChannel = it })
                            Text(if (isChannel) "Канал" else "Группа")
                        }
                    }
                }, confirmButton = {
                    TextButton(onClick = {
                        scope.launch {
                            runCatching {
                                Core.call("create_channel", mapOf("title" to title, "about" to "", "broadcast" to isChannel))
                                showCreate = false
                                reload()
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text("Создать") }
                }, dismissButton = { TextButton(onClick = { showCreate = false }) { Text("Отмена") } })
            }
            IconButton(onClick = {
                scope.launch {
                    runCatching {
                        val r = Core.call("dialog_filters", emptyMap())
                        folders = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                        showFolders = true
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("📁") }
            IconButton(onClick = { showNearby = !showNearby }) { Text("📍") }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { nav.navigate("downloads") }) { Icon(Icons.Default.Menu, "Загрузки") }
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        OutlinedTextField(query, {
            query = it
            scope.launch {
                // Глобальный поиск сервера от 3 символов (результат — в Журнале/статусе).
                runCatching {
                    if (it.length >= 3) {
                        val r = Core.call("global_search", mapOf("query" to it, "limit" to 20))
                        val arr = r.getJSONArray("result")
                        gsearch = (0 until arr.length()).map { k ->
                            val el = arr.get(k)
                            if (el is org.json.JSONArray) Pair(el.optLong(0), el.optJSONObject(1))
                            else { val o = el as org.json.JSONObject; Pair(o.optLong("chat"), o.optJSONObject("message")) }
                        }
                        if (gsearch.isNotEmpty()) showSearch = true
                    }
                }
            }
        }, label = { Text("Поиск (сервер от 3 символов)") }, modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp))
        if (showNearby) {
            Card(Modifier.fillMaxWidth().padding(8.dp)) {
                Column(Modifier.padding(8.dp)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(lat, { lat = it }, label = { Text("lat") }, modifier = Modifier.weight(1f))
                        OutlinedTextField(long, { long = it }, label = { Text("long") }, modifier = Modifier.weight(1f))
                        Button(onClick = {
                            scope.launch {
                                runCatching {
                                    val r = Core.call("nearby", mapOf("lat" to (lat.toDoubleOrNull() ?: 0.0), "long" to (long.toDoubleOrNull() ?: 0.0)))
                                    nearby = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("↻") }
                    }
                    nearby.forEach { p ->
                        Text("${if (p.optBoolean("is_user")) "👤" else "👥"} ${p.optString("title")}")
                    }
                }
            }
        }
        LazyColumn(Modifier.weight(1f)) {
            itemsIndexed(chats.filter { query.isEmpty() || it.title.contains(query, true) }, key = { i, c -> "chat-$i-${c.id}" }) { _, c ->
                ListItem(
                    headlineContent = { Text(c.title) },
                    supportingContent = { Text("unread=${c.unread}${if (c.muted) " · muted" else ""}") },
                    modifier = Modifier.clickable { nav.navigate("chat/${c.id}") },
                    trailingContent = {
                        Row {
                            IconButton(onClick = { scope.launch { runCatching { Core.call("archive", mapOf("chat" to c.id, "archived" to !c.archived)); reload() } } }) { Text(if (c.archived) "📥" else "🗄") }
                            IconButton(onClick = { scope.launch { runCatching { Core.call("pin_dialog", mapOf("chat" to c.id, "pinned" to true)); reload() } } }) { Text("📌") }
                            IconButton(onClick = { scope.launch { runCatching { Core.call("mute", mapOf("chat" to c.id, "muted" to !c.muted)); reload() } } }) { Text(if (c.muted) "🔔" else "🔕") }
                        }
                    },
                )
                HorizontalDivider()
            }
        }
        if (status.isNotEmpty()) Text(status, modifier = Modifier.padding(8.dp), style = MaterialTheme.typography.bodySmall)
        if (showSearch) {
            AlertDialog(onDismissRequest = { showSearch = false }, title = { Text("Глобальный поиск") }, text = {
                LazyColumn(Modifier.heightIn(max = 400.dp)) {
                    items(gsearch.size) { i ->
                        val (chatId, msg) = gsearch[i]
                        Column(Modifier.clickable {
                            if (chatId != 0L) nav.navigate("chat/$chatId")
                            showSearch = false
                        }) {
                            Text("chat $chatId", style = MaterialTheme.typography.bodySmall)
                            if (msg != null) Text(msg.optString("text", "").take(100), style = MaterialTheme.typography.bodySmall)
                            HorizontalDivider()
                        }
                    }
                }
            }, confirmButton = { TextButton(onClick = { showSearch = false }) { Text("Закрыть") } })
        }
        renameTarget?.let { rf ->
        AlertDialog(onDismissRequest = { renameTarget = null }, title = { Text("✏️ Переименовать") }, text = {
            OutlinedTextField(renameTitle, { renameTitle = it }, label = { Text("Новое название") })
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("rename_folder", mapOf("id" to rf.optInt("id"), "title" to renameTitle))
                        renameTarget = null
                        val r = Core.call("dialog_filters", emptyMap())
                        folders = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Сохранить") }
        }, dismissButton = { TextButton(onClick = { renameTarget = null }) { Text("Отмена") } })
    }
    if (showFolders) {
            var title by remember { mutableStateOf("") }
            AlertDialog(onDismissRequest = { showFolders = false }, title = { Text("📁 Папки") }, text = {
                Column {
                    folders.forEach { f ->
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Text("• ${f.optString("title", "?")}", modifier = Modifier.weight(1f),
                                style = MaterialTheme.typography.bodySmall)
                        TextButton(onClick = { renameTarget = f; renameTitle = f.optString("title", "") }) { Text("✏️") }
                        TextButton(onClick = {
                            scope.launch {
                                runCatching {
                                    Core.call("delete_folder", mapOf("id" to f.optInt("id")))
                                    val r = Core.call("dialog_filters", emptyMap())
                                    folders = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        }) { Text("✕") }
                        }
                    }
                    OutlinedTextField(title, { title = it }, label = { Text("Новая папка") })
                }
            }, confirmButton = {
                TextButton(onClick = {
                    scope.launch {
                        runCatching {
                            if (title.isNotBlank()) {
                                Core.call("create_folder", mapOf("title" to title, "peers" to emptyList<Long>()))
                                title = ""
                                val r = Core.call("dialog_filters", emptyMap())
                                folders = (0 until r.getJSONArray("result").length()).map { r.getJSONArray("result").getJSONObject(it) }
                            } else showFolders = false
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }) { Text("Создать") }
            }, dismissButton = { TextButton(onClick = { showFolders = false }) { Text("Закрыть") } })
        }
    }
}
