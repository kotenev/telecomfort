package com.telecomfort

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.*
import com.telecomfort.core.SecureStore
import com.telecomfort.ui.AppNav
import com.telecomfort.ui.theme.TelecomfortTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            val store = remember { SecureStore(this) }
            // Тема из шифрохранилища: dark | light | system (дефолт).
            var theme by remember { mutableStateOf(store.theme) }
            val dark = when (theme) {
                "light" -> false
                "dark" -> true
                else -> isSystemInDarkTheme()
            }
            TelecomfortTheme(dark = dark) {
                AppNav(
                    onToggleTheme = {
                        theme = if (theme == "dark") "light" else "dark"
                        store.theme = theme
                    },
                    onThemeMode = { mode ->
                        theme = mode
                        store.theme = mode
                    },
                    currentTheme = theme,
                )
            }
        }
    }
}
