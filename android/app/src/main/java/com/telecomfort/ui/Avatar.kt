package com.telecomfort.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.telecomfort.core.Core
import com.telecomfort.core.optStr
import kotlinx.coroutines.launch

/** Кэш аватаров в памяти: chatId -> Bitmap (отсутствие записи = ещё не пробовали). */
object AvatarCache {
    val bitmaps = mutableMapOf<Long, Bitmap?>()
    var failed = mutableSetOf<Long>()
}

/**
 * Аватар чата: скачивание через мост (backend.avatar_path резолвит фото сам),
 * кэш в памяти, фолбэк — первая буква названия.
 */
@Composable
fun AvatarImage(chatId: Long, title: String, size: Dp = 40.dp, onClick: (() -> Unit)? = null) {
    var bmp by remember(chatId) { mutableStateOf(AvatarCache.bitmaps[chatId]) }
    var requested by remember(chatId) { mutableStateOf(AvatarCache.bitmaps.containsKey(chatId) || AvatarCache.failed.contains(chatId)) }
    val scope = rememberCoroutineScope()
    if (bmp == null && !requested) {
        requested = true
        LaunchedEffect(chatId) {
            scope.launch {
                runCatching {
                    val r = Core.call("avatar_path", mapOf("chat" to chatId))
                    val path = r.optStr("path")
                    val b = if (!path.isNullOrEmpty()) BitmapFactory.decodeFile(path) else null
                    if (b != null) {
                        AvatarCache.bitmaps[chatId] = b
                        bmp = b
                    } else {
                        AvatarCache.failed.add(chatId)
                    }
                }.onFailure { AvatarCache.failed.add(chatId) }
            }
        }
    }
    var mod = Modifier.size(size).clip(CircleShape)
    if (onClick != null) {
        mod = mod.clickable(indication = null, interactionSource = remember { MutableInteractionSource() }, onClick = onClick)
    }
    if (bmp != null) {
        Image(bmp!!.asImageBitmap(), contentDescription = title, modifier = mod)
    } else {
        Box(mod.background(MaterialTheme.colorScheme.primaryContainer), contentAlignment = Alignment.Center) {
            Text((title.firstOrNull() ?: "?").toString(), style = MaterialTheme.typography.titleMedium)
        }
    }
}
