package com.telecomfort.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val Dark = darkColorScheme(
    primary = Color(0xFF5EEAD4), background = Color(0xFF0B1220),
    surface = Color(0xFF111A2E), onBackground = Color(0xFFE2E8F0),
)
private val Light = lightColorScheme(
    primary = Color(0xFF0E7490), background = Color(0xFFF8FAFC),
    surface = Color(0xFFFFFFFF), onBackground = Color(0xFF0F172A),
)

@Composable
fun TelecomfortTheme(dark: Boolean, content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (dark) Dark else Light, content = content)
}
