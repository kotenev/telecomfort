package com.telecomfort.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.telecomfort.core.Core
import kotlinx.coroutines.launch
import org.json.JSONObject

/** ⭐ Баланс, история транзакций, мои подарки, каталог + отправка. */
@Composable
fun StarsScreen(nav: NavController) {
    var balance by remember { mutableStateOf<Long?>(null) }
    var txs by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var saved by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var catalog by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var status by remember { mutableStateOf("") }
    var chatId by remember { mutableStateOf("") }
    var giftMsg by remember { mutableStateOf("") }
    var armed by remember { mutableStateOf<Long?>(null) }
    val scope = rememberCoroutineScope()

    fun reload() {
        scope.launch {
            runCatching {
                val b = Core.call("stars_balance")
                balance = b.optLong("amount", b.optLong("stars", 0))
                val t = Core.call("stars_transactions", mapOf("inbound" to true, "outbound" to true, "limit" to 50))
                txs = (0 until t.getJSONArray("result").length()).map { t.getJSONArray("result").getJSONObject(it) }
                val g = Core.call("saved_star_gifts")
                saved = (0 until g.getJSONArray("result").length()).map { g.getJSONArray("result").getJSONObject(it) }
                val c = Core.call("gift_catalog")
                catalog = (0 until c.getJSONArray("result").length()).map { c.getJSONArray("result").getJSONObject(it) }
            }.onFailure { status = "Ошибка: ${it.message}" }
        }
    }
    LaunchedEffect(Unit) { reload() }

    Column(Modifier.fillMaxSize().padding(8.dp)) {
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Назад") }
            Text("⭐ Баланс: ${balance ?: "…"}", style = MaterialTheme.typography.headlineSmall)
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        Text("Транзакции:", style = MaterialTheme.typography.titleSmall)
        LazyColumn(Modifier.weight(1f)) {
            items(txs.take(30)) { t ->
                val s = t.optLong("stars")
                Text("${if (s >= 0) "+" else ""}$s ⭐ · ${t.optString("title", "")} ${t.optString("peer_name", "")}",
                    style = MaterialTheme.typography.bodySmall)
            }
        }
        Text("Мои подарки:", style = MaterialTheme.typography.titleSmall)
        saved.take(10).forEach { g ->
            Text("🎁 ${g.optString("title")} — ⭐ ${g.optLong("stars")} от ${g.optString("from_name", "?")}",
                style = MaterialTheme.typography.bodySmall)
        }
        Text("Отправить подарок:", style = MaterialTheme.typography.titleSmall)
        OutlinedTextField(chatId, { chatId = it }, label = { Text("ID чата получателя") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(giftMsg, { giftMsg = it }, label = { Text("Текст (необязательно)") }, modifier = Modifier.fillMaxWidth())
        LazyColumn(Modifier.height(160.dp)) {
            items(catalog) { g ->
                val id = g.optLong("id")
                Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                    Text("⭐ ${g.optLong("stars")} — ${g.optString("title")}", modifier = Modifier.weight(1f))
                    if (g.optBoolean("sold_out")) Text("распродан", style = MaterialTheme.typography.bodySmall)
                    else TextButton(onClick = {
                        if (armed == id) {
                            armed = null
                            scope.launch {
                                runCatching {
                                    val r = Core.call("send_star_gift", mapOf(
                                        "chat" to (chatId.toLongOrNull() ?: 0L), "gift" to id, "message" to giftMsg))
                                    status = r.optString("summary", "отправлено")
                                    reload()
                                }.onFailure { status = "Ошибка: ${it.message}" }
                            }
                        } else {
                            armed = id
                            status = "⚠ Нажмите ещё раз — спишутся звёзды"
                        }
                    }) { Text(if (armed == id) "⚠ Подарить!" else "Подарить") }
                }
            }
        }
        if (status.isNotEmpty()) Text(status, style = MaterialTheme.typography.bodySmall)
    }
}
