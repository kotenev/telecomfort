package com.telecomfort.ui.screens

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.telecomfort.core.Core
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject

/**
 * Переключатель динамика для звонка. На API 31+ используем CommunicationDevice,
 * на старых — legacy isSpeakerphoneOn (подавлено точечно ради minSdk 26).
 */
private fun AudioManager.setCallSpeaker(enabled: Boolean) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        if (enabled) {
            val speaker = availableCommunicationDevices.firstOrNull {
                it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER
            }
            if (speaker == null || !setCommunicationDevice(speaker)) {
                @Suppress("DEPRECATION")
                isSpeakerphoneOn = true
            }
        } else {
            clearCommunicationDevice()
        }
        return
    }
    @Suppress("DEPRECATION")
    isSpeakerphoneOn = enabled
}

/**
 * Экран 1:1 звонка. Медиапоток (libtgvoip + SRTP-подобное шифрование,
 * Opus, jitter buffer) живёт в Rust; здесь — сигналинг и управление.
 */
@Composable
fun CallScreen(nav: NavController) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val am = remember { ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager }

    var call by remember { mutableStateOf<JSONObject?>(null) }
    var muted by remember { mutableStateOf(false) }
    var speaker by remember { mutableStateOf(false) }
    var secs by remember { mutableStateOf(0) }
    var stats by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var gone by remember { mutableStateOf(false) }
    val micPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (!ok) status = "Нет доступа к микрофону"
    }

    LaunchedEffect(Unit) {
        if (androidx.core.content.ContextCompat.checkSelfPermission(
                ctx, Manifest.permission.RECORD_AUDIO
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            micPerm.launch(Manifest.permission.RECORD_AUDIO)
        }
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        muted = false
        while (!gone) {
            runCatching {
                val r = Core.call("call_state")
                call = r.optJSONObject("call")
                val state = call?.optString("state") ?: ""
                if (state == "ended" || (state.isEmpty() && call == null)) {
                    gone = true
                }
                if (state == "active") {
                    secs++
                    runCatching {
                        val s = Core.call("call_stats")
                        stats = "${s.optLong("sent") / 1024} КБ ↑ · ${s.optLong("received") / 1024} КБ ↓"
                    }
                }
            }
            if (gone) {
                nav.popBackStack()
                break
            }
            delay(1000)
        }
    }
    DisposableEffect(Unit) {
        onDispose {
            am.setCallSpeaker(false)
            am.mode = AudioManager.MODE_NORMAL
        }
    }

    val state = call?.optString("state") ?: "waiting"
    val video = call?.optBoolean("video") == true
    val peer = call?.optLong("peer_id") ?: 0
    val err = call?.optString("error").orEmpty().let { if (it == "null") "" else it }

    Surface(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Spacer(Modifier.weight(0.6f))
            com.telecomfort.ui.AvatarImage(peer, "call")
            Text(
                when (state) {
                    "ringing" -> "Входящий звонок"
                    "waiting" -> "Звоним…"
                    "active" -> "Разговор"
                    else -> state
                },
                style = MaterialTheme.typography.headlineSmall,
            )
            if (video) Text("📹 видео", style = MaterialTheme.typography.bodySmall)
            if (state == "active") {
                val mm = secs / 60
                val ss = secs % 60
                Text("%02d:%02d".format(mm, ss), style = MaterialTheme.typography.displaySmall)
                Text(stats, style = MaterialTheme.typography.bodySmall)
            }
            if (err.isNotEmpty()) Text("⚠ $err", color = MaterialTheme.colorScheme.error)
            if (status.isNotEmpty()) Text(status, style = MaterialTheme.typography.bodySmall)
            Spacer(Modifier.weight(1f))

            when (state) {
                "ringing" -> Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                    Button(onClick = {
                        val id = call?.optLong("call_id") ?: 0
                        val hash = call?.optLong("access_hash") ?: 0
                        scope.launch {
                            runCatching {
                                Core.call("call_accept", mapOf("call_id" to id, "access_hash" to hash))
                                secs = 0
                            }.onFailure { status = "Не удалось принять: ${it.message}" }
                        }
                    }) { Text("✅ Принять") }
                    OutlinedButton(onClick = {
                        val id = call?.optLong("call_id") ?: 0
                        val hash = call?.optLong("access_hash") ?: 0
                        scope.launch {
                            runCatching {
                                Core.call("decline_call", mapOf("call_id" to id, "access_hash" to hash))
                                gone = true
                                nav.popBackStack()
                            }.onFailure { status = "Ошибка: ${it.message}" }
                        }
                    }) { Text("✕ Отклонить") }
                }
                "waiting" -> Button(onClick = {
                    scope.launch {
                        runCatching { Core.call("call_hangup") }
                        gone = true
                        nav.popBackStack()
                    }
                }) { Text("Отменить") }
                "active" -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        FilledTonalButton(onClick = {
                            muted = !muted
                            scope.launch { runCatching { Core.call("call_mute", mapOf("muted" to muted)) } }
                        }) { Text(if (muted) "🔇 Микрофон выкл" else "🎤 Микрофон вкл") }
                        FilledTonalButton(onClick = {
                            speaker = !speaker
                            am.setCallSpeaker(speaker)
                        }) { Text(if (speaker) "🔊 Динамик" else "📞 Трубка") }
                    }
                    Spacer(Modifier.height(16.dp))
                    Button(
                        onClick = {
                            scope.launch {
                                runCatching { Core.call("call_hangup") }
                                gone = true
                                nav.popBackStack()
                            }
                        },
                        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
                    ) { Text("Завершить") }
                }
            }
            Spacer(Modifier.weight(0.4f))
        }
    }
}
