package com.telecomfort.ui

import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.telecomfort.core.Core
import com.telecomfort.ui.screens.*
import kotlinx.coroutines.launch

@Composable
fun AppNav(onToggleTheme: () -> Unit) {
    val ctx = LocalContext.current
    val store = remember { com.telecomfort.core.SecureStore(ctx) }
    // PIN задан → стартуем заблокированными.
    var locked by remember { mutableStateOf(store.pinSha256.isNotEmpty()) }
    com.telecomfort.core.LockManager.locked = locked
    // Автоблок: проверка каждые 5с.
    LaunchedEffect(Unit) {
        while (true) {
            kotlinx.coroutines.delay(5000)
            if (!locked && store.pinSha256.isNotEmpty()
                && com.telecomfort.core.LockManager.due(store.autoLockSecs)) {
                locked = true
                com.telecomfort.core.LockManager.locked = true
            }
        }
    }
    if (locked) {
        com.telecomfort.ui.screens.LockScreen(store) { locked = false }
        return
    }
    val nav = rememberNavController()
    var authed by remember { mutableStateOf<Boolean?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        // Демо-режим сразу; live — после входа.
        scope.launch {
            runCatching { Core.call("use_mock") }
            authed = true
        }
    }
    if (authed == null) return
    NavHost(navController = nav, startDestination = "login") {
        composable("login") { LoginScreen(onDone = { nav.navigate("chats") }, store = store) }
        composable("chats") { ChatsScreen(nav) }
        composable("chat/{id}") { back ->
            ChatScreen(chatId = back.arguments?.getString("id")?.toLongOrNull() ?: 0L, nav)
        }
        composable("stories") { StoriesScreen(nav) }
        composable("stars") { StarsScreen(nav) }
        composable("contacts") { ContactsScreen(nav) }
        composable("settings") { SettingsScreen(nav, onToggleTheme, store, onLockNow = { locked = true }) }
    }
}
