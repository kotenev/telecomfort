package com.telecomfort

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.*
import com.telecomfort.core.SecureStore
import com.telecomfort.ui.AppNav
import com.telecomfort.ui.theme.TelecomfortTheme

class MainActivity : ComponentActivity() {
    override fun onStart() {
        super.onStart()
        com.telecomfort.core.AppVisibility.visible = true
    }

    override fun onStop() {
        com.telecomfort.core.AppVisibility.visible = false
        super.onStop()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            val store = remember { SecureStore(this) }
            val notifPerm = rememberLauncherForActivityResult(
                ActivityResultContracts.RequestPermission()
            ) { }
            LaunchedEffect(Unit) {
                if (Build.VERSION.SDK_INT >= 33 &&
                    checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                ) {
                    notifPerm.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
            }
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
