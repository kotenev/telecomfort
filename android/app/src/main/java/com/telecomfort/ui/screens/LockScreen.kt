package com.telecomfort.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.telecomfort.core.LockManager
import com.telecomfort.core.SecureStore

/** PIN-экран: перекрывает всё до верного PIN (SHA-256 сверка). */
@Composable
fun LockScreen(store: SecureStore, onUnlock: () -> Unit) {
    var pin by remember { mutableStateOf("") }
    var error by remember { mutableStateOf("") }
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text("🔒 Telecomfort заблокирован", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(16.dp))
        OutlinedTextField(
            pin, { pin = it }, label = { Text("PIN") },
            visualTransformation = PasswordVisualTransformation(),
            singleLine = true,
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = {
            if (SecureStore.sha256Hex(pin) == store.pinSha256) {
                pin = ""
                LockManager.locked = false
                LockManager.touch()
                onUnlock()
            } else {
                pin = ""
                error = "Неверный PIN"
            }
        }) { Text("Разблокировать") }
        if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
    }
}
