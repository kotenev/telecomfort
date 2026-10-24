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
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Контакты: список, добавить по телефону, удалить, блокировка, чёрный список. */
@Composable
fun ContactsScreen(nav: NavController) {
    var contacts by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var blocked by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var showBlocked by remember { mutableStateOf(false) }
    var showAdd by remember { mutableStateOf(false) }
    var phone by remember { mutableStateOf("") }
    var first by remember { mutableStateOf("") }
    var last by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()

    fun reload() {
        scope.launch {
            runCatching {
                val c = Core.call("contacts")
                contacts = (0 until c.getJSONArray("result").length()).map { c.getJSONArray("result").getJSONObject(it) }
                val b = Core.call("blocked_list")
                blocked = (0 until b.getJSONArray("result").length()).map { b.getJSONArray("result").getJSONObject(it) }
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(Unit) { reload() }

    Column(Modifier.fillMaxSize().padding(8.dp)) {
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
            Text(if (showBlocked) "🚫 Чёрный список" else "👥 Контакты", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            TextButton(onClick = { showBlocked = !showBlocked }) { Text(if (showBlocked) "Контакты" else "🚫") }
            if (!showBlocked) TextButton(onClick = { showAdd = true }) { Text("＋") }
        }
        if (!showBlocked) {
            LazyColumn(Modifier.weight(1f)) {
                itemsIndexed(contacts, key = { i, c -> "c-$i-${c.optLong("user_id")}" }) { _, c ->
                    val name = c.optString("name", "").ifEmpty { c.optString("phone", "?") }
                    ListItem(
                        headlineContent = { Text(name) },
                        supportingContent = { Text(c.optString("phone", "")) },
                        trailingContent = {
                            Row {
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            Core.call("block_user", mapOf("chat" to c.optLong("user_id", c.optLong("id")), "blocked" to true))
                                            status = "Заблокирован"
                                            reload()
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("🚫") }
                                TextButton(onClick = {
                                    scope.launch {
                                        runCatching {
                                            Core.call("delete_contact", mapOf("user" to c.optLong("user_id", c.optLong("id")), "hash" to c.optLong("access_hash")))
                                            status = "Удалён"
                                            reload()
                                        }.onFailure { status = "Ошибка: ${it.message}" }
                                    }
                                }) { Text("🗑") }
                            }
                        },
                    )
                    HorizontalDivider()
                }
            }
        } else {
            LazyColumn(Modifier.weight(1f)) {
                itemsIndexed(blocked, key = { i, b -> "b-$i-${b.optLong("id")}" }) { _, b ->
                    ListItem(
                        headlineContent = { Text(b.optString("title", b.optString("name", "?"))) },
                        trailingContent = {
                            TextButton(onClick = {
                                scope.launch {
                                    runCatching {
                                        Core.call("block_user", mapOf("chat" to b.optLong("id", b.optLong("user_id")), "blocked" to false))
                                        status = "Разблокирован"
                                        reload()
                                    }.onFailure { status = "Ошибка: ${it.message}" }
                                }
                            }) { Text("✓") }
                        },
                    )
                    HorizontalDivider()
                }
            }
        }
        if (status.isNotEmpty()) Text(status, style = MaterialTheme.typography.bodySmall)
    }
    if (showAdd) {
        AlertDialog(onDismissRequest = { showAdd = false }, title = { Text("＋ Новый контакт") }, text = {
            Column {
                OutlinedTextField(phone, { phone = it }, label = { Text("Телефон") })
                OutlinedTextField(first, { first = it }, label = { Text("Имя") })
                OutlinedTextField(last, { last = it }, label = { Text("Фамилия") })
            }
        }, confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    runCatching {
                        Core.call("add_contact", mapOf("phone" to phone, "first" to first, "last" to last))
                        showAdd = false
                        reload()
                    }.onFailure { status = "Ошибка: ${it.message}" }
                }
            }) { Text("Добавить") }
        }, dismissButton = { TextButton(onClick = { showAdd = false }) { Text("Отмена") } })
    }
}
