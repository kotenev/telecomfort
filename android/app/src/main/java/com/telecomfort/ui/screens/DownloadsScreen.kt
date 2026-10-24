package com.telecomfort.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.telecomfort.core.Core
import kotlinx.coroutines.launch
import java.io.File

/** Загрузки: файлы, скачанные мостом (медиа чатов/сторис, экспорт, войсы). */
@Composable
fun DownloadsScreen(nav: NavController) {
    val ctx = LocalContext.current
    var files by remember { mutableStateOf<List<File>>(emptyList()) }
    var status by remember { mutableStateOf("") }
    var groups by remember { mutableStateOf<List<List<File>>>(emptyList()) }
    var showDups by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun reload() {
        val dirs = listOf(
            File(ctx.filesDir, "downloads"),
            ctx.cacheDir,
        )
        files = dirs.flatMap { d ->
            d.walkTopDown().maxDepth(2).filter { it.isFile }.toList()
        }.sortedByDescending { it.lastModified() }.take(300)
        val total = files.sumOf { it.length() }
        status = "Файлов: ${files.size}, всего ${"%.1f".format(total / 1048576.0)} МБ"
    }
    LaunchedEffect(Unit) { reload() }

    Column(Modifier.fillMaxSize().padding(8.dp)) {
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Назад") }
            Text("⬇ Загрузки", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
            IconButton(onClick = {
                showDups = !showDups
                if (showDups) {
                    scope.launch {
                        runCatching {
                            val r = Core.call("local_duplicates", mapOf(
                                "dirs" to listOf(
                                    File(ctx.filesDir, "downloads").absolutePath,
                                    ctx.cacheDir.absolutePath,
                                ),
                                "threshold" to 6,
                            ))
                            val arr = r.getJSONArray("groups")
                            groups = (0 until arr.length()).map { gi ->
                                val g = arr.getJSONArray(gi)
                                (0 until g.length()).map { File(g.getString(it)) }
                            }
                            status = "Похожих групп: ${groups.size} (${r.optInt("scanned")} картинок)"
                        }.onFailure { status = "Ошибка: ${it.message}" }
                    }
                }
            }) { Icon(Icons.Default.Search, "Похожие") }
        }
        Text(status, style = MaterialTheme.typography.bodySmall)
        if (showDups) {
            if (groups.isEmpty()) {
                Text("Похожих картинок не найдено", style = MaterialTheme.typography.bodySmall)
            } else {
                LazyColumn(Modifier.heightIn(max = 240.dp)) {
                    itemsIndexed(groups, key = { i, g -> "dup-$i-${g.firstOrNull()?.name}" }) { _, g ->
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Column(Modifier.weight(1f)) {
                                Text("≈ ${g.size} файл(ов)", style = MaterialTheme.typography.bodySmall)
                                Text(g.joinToString(", ") { it.name.take(24) },
                                    style = MaterialTheme.typography.labelSmall, maxLines = 2)
                            }
                            TextButton(onClick = {
                                var removed = 0
                                g.drop(1).forEach { if (it.delete()) removed++ }
                                status = "Удалено дублей: $removed"
                                reload()
                                groups = groups.filter { it !== g }
                            }) { Text("🗑 дубли") }
                        }
                        HorizontalDivider()
                    }
                }
            }
        }
        LazyColumn(Modifier.weight(1f)) {
            itemsIndexed(files, key = { i, f -> "dl-$i-${f.name}" }) { _, f ->
                ListItem(
                    headlineContent = { Text(f.name.take(48)) },
                    supportingContent = { Text("${f.length() / 1024} КБ · ${f.parentFile?.name}") },
                    trailingContent = {
                        TextButton(onClick = {
                            try {
                                val uri = androidx.core.content.FileProvider.getUriForFile(ctx, ctx.packageName + ".files", f)
                                val intent = android.content.Intent(android.content.Intent.ACTION_VIEW).apply {
                                    setDataAndType(uri, ctx.contentResolver.getType(uri) ?: "*/*")
                                    addFlags(android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                }
                                ctx.startActivity(android.content.Intent.createChooser(intent, "Открыть"))
                            } catch (e: Exception) { status = "Нечем открыть: ${e.message}" }
                        }) { Text("↗") }
                        TextButton(onClick = {
                            if (f.delete()) reload() else status = "Не удалён: ${f.name}"
                        }) { Text("🗑") }
                    },
                )
                HorizontalDivider()
            }
        }
    }
}
