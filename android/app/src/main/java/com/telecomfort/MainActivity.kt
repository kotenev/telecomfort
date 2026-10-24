package com.telecomfort

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.*
import com.telecomfort.ui.AppNav
import com.telecomfort.ui.theme.TelecomfortTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            var dark by remember { mutableStateOf(true) }
            // TODO: читать тему из настроек (DataStore).
            TelecomfortTheme(dark = dark) {
                AppNav(onToggleTheme = { dark = !dark })
            }
        }
    }
}
