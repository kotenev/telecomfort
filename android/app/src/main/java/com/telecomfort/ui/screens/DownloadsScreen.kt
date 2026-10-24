package com.telecomfort.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import java.io.File

/** Загрузки: файлы, скачанные мостом (медиа чатов/сторис, экспорт, войсы). */
@Composable
fun DownloadsScreen(nav: NavController) {
    val ctx = LocalContext.current
    var files by remember { mutableStateOf<List<File>>(emptyList()) }
    var status by remember { mutableStateOf("") }

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
            IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Назад") }
            Text("⬇ Загрузки", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
            IconButton(onClick = { reload() }) { Icon(Icons.Default.Refresh, "Обновить") }
        }
        Text(status, style = MaterialTheme.typography.bodySmall)
        LazyColumn(Modifier.weight(1f)) {
            itemsIndexed(files, key = { i, f -> "dl-$i-${f.name}" }) { _, f ->
                ListItem(
                    headlineContent = { Text(f.name.take(48)) },
                    supportingContent = { Text("${f.length() / 1024} КБ · ${f.parentFile?.name}") },
                    trailingContent = {
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
