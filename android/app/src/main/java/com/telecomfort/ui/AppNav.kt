package com.telecomfort.ui

import androidx.compose.runtime.*
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.telecomfort.core.Core
import com.telecomfort.ui.screens.*
import kotlinx.coroutines.launch

@Composable
fun AppNav(onToggleTheme: () -> Unit) {
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
        composable("login") { LoginScreen(onDone = { nav.navigate("chats") }) }
        composable("chats") { ChatsScreen(nav) }
        composable("chat/{id}") { back ->
            ChatScreen(chatId = back.arguments?.getString("id")?.toLongOrNull() ?: 0L, nav)
        }
        composable("stories") { StoriesScreen(nav) }
        composable("stars") { StarsScreen(nav) }
        composable("settings") { SettingsScreen(nav, onToggleTheme) }
    }
}
