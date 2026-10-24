//! Telecomfort core: Telegram-слой, медиа, AI, БД, синк, интеграции.
//!
//! Desktop-битарь (`src/main.rs`) — тонкая обёртка: tokio-рантайм + eframe.
//! Android (`--target aarch64-linux-android`) собирает этот крейт как cdylib;
//! JNI-мост — `android_bridge` (только Android), Kotlin UI — в `android/`.

pub mod ai;
pub mod config;
pub mod db;
pub mod events;
pub mod integrations;
pub mod media;
pub mod sync;
pub mod telegram;
#[cfg(not(target_os = "android"))]
pub mod ui;
pub mod utils;

/// JNI-мост Android (JSON-протокол). Чистая логика тестируется на хосте;
/// JNI-экспорт — только на Android.
pub mod android_bridge;
