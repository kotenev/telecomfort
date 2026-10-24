package com.telecomfort.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.telecomfort.core.Core
import com.telecomfort.core.LockManager
import com.telecomfort.core.SecureStore
import com.telecomfort.ui.screens.*
import kotlinx.coroutines.launch

@Composable
fun AppNav(onToggleTheme: () -> Unit = {}, onThemeMode: (String) -> Unit = {}, currentTheme: String = "system") {
    val ctx = LocalContext.current
    val store = remember { SecureStore(ctx) }
    var locked by remember { mutableStateOf(store.pinSha256.isNotEmpty()) }
    LockManager.locked = locked
    LaunchedEffect(Unit) {
        while (true) {
            kotlinx.coroutines.delay(5000)
            if (!locked && store.pinSha256.isNotEmpty() && LockManager.due(store.autoLockSecs)) {
                locked = true
                LockManager.locked = true
            }
        }
    }
    if (locked) {
        LockScreen(store) { locked = false }
        return
    }
    // Глобальный трекер активности: любое касание сбрасывает таймер автоблока.
    Box(
        Modifier.fillMaxSize().pointerInput(Unit) {
            awaitPointerEventScope {
                while (true) {
                    awaitPointerEvent(PointerEventPass.Initial)
                    LockManager.touch()
                }
            }
        }
    ) {
        AppNavHost(onToggleTheme, store, onLock = { locked = true }, onThemeMode = onThemeMode, currentTheme = currentTheme)
    }
}

@Composable
private fun AppNavHost(onToggleTheme: () -> Unit, store: SecureStore, onLock: () -> Unit, onThemeMode: (String) -> Unit = {}, currentTheme: String = "system") {
    val ctx = LocalContext.current
    val nav = rememberNavController()
    var authed by remember { mutableStateOf<Boolean?>(null) }
    val scope = rememberCoroutineScope()
    var startDestination by remember { mutableStateOf("login") }
    LaunchedEffect(Unit) {
        scope.launch {
            // Автологин: сохранённые ключи + живая сессия — сразу в чаты.
            val ok = runCatching {
                val aid = store.apiId.toLongOrNull() ?: 0L
                if (aid == 0L || store.apiHash.isBlank()) return@runCatching false
                val filesDir = ctx.filesDir.absolutePath
                Core.call("init", mapOf("dir" to filesDir, "api_id" to aid,
                    "api_hash" to store.apiHash, "proxy" to store.proxy))
                Core.call("is_authorized", emptyMap()).optBoolean("authorized", false)
            }.getOrDefault(false)
            if (ok) {
                startDestination = "chats"
                com.telecomfort.core.CoreBootstrap.markReady()
                com.telecomfort.core.RealtimeService.start(ctx)
            } else runCatching { Core.call("use_mock") }
            authed = true
        }
    }
    if (authed == null) return
    // Входящий/идущий звонок: поднимаем экран звонка из любого места.
    LaunchedEffect(Unit) {
        while (true) {
            if (store.apiId.isNotEmpty()) {
                runCatching {
                    val c = Core.call("call_state").optJSONObject("call")
                    val st = c?.optString("state") ?: ""
                    if (st in setOf("ringing", "waiting", "active")) {
                        val route = nav.currentBackStackEntry?.destination?.route
                        if (route != "call") nav.navigate("call")
                    }
                }
            }
            kotlinx.coroutines.delay(1500)
        }
    }
    NavHost(navController = nav, startDestination = startDestination) {
        composable("login") { LoginScreen(onDone = { nav.navigate("chats") }, store = store) }
        composable("chats") { ChatsScreen(nav) }
        composable("call") { CallScreen(nav) }
        composable("chat/{id}") { back ->
            ChatScreen(chatId = back.arguments?.getString("id")?.toLongOrNull() ?: 0L, nav)
        }
        composable("stories") { StoriesScreen(nav, store) }
        composable("stars") { StarsScreen(nav) }
        composable("contacts") { ContactsScreen(nav) }
        composable("downloads") { DownloadsScreen(nav) }
        composable("settings") {
                SettingsScreen(nav, onToggleTheme, store,
                    onLockNow = onLock, onThemeMode = onThemeMode, currentTheme = currentTheme,
                    onLogout = { nav.navigate("login") { popUpTo(0) } })
            }
    }
}
