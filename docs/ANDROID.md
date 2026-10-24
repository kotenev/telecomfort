# Telecomfort Android

Нативное Android-приложение: Kotlin + Jetpack Compose UI поверх **того же
Rust-ядра**, что и десктоп. Вся бизнес-логика (MTProto, медиа, AI-детект,
Яндекс.Диск, экспорт) — общая, UI — нативный.

## Архитектура

```
android/app (Kotlin/Compose)          Rust core (cdylib libtelecomfort.so)
  Login/Chats/Chat/                    src/android_bridge.rs
  Stories/Stars/Settings    -- JSON --> TelegramService (тот же трейт!)
  Core.nativeCall(cmd): String <-------
```

- **Мост**: один JNI-метод `Core.nativeCall(cmdJson): String`, протокол
  `{"op":...,"param":...}` → `{"ok":true,...}`. ~90 операций — весь трейт.
  Чистая логика (`core_call`) тестируется на хосте (`cargo test --lib bridge_`).
- **Состояние**: `BridgeCore { tg: Option<Arc<dyn TelegramService>> }`,
  свой Tokio-рантайм внутри .so; сессии — в `filesDir/session_android.session`.
- **Медиа**: пути на устройстве; превью — BitmapFactory из локальных файлов.
- **Голос**: запись — `MediaRecorder` (Kotlin, 48кГц моно) → WAV →
  `wav_to_ogg` (Opus в Rust) → `send_voice` (настоящие войсы!).
- **Сторис**: `ad_score` (эвристики) → бейдж + причины; `qr_hint` — QR-маркер;
  `blur_full` — блюр; `yandex_upload` — сохранение в Яндекс.Диск.
- **Защита от копирования**: бэкенд по контракту игнорирует `noforwards`
  (см. `TelegramService::download_message_media`), поэтому текст/медиа
  защищённых чатов доступны в приложении — как и в десктопе.

## Сборка

```bash
# 1. Rust .so (нужны: rustup stable + android-таргеты, NDK r27, autoconf/automake):
./android/build-rust.sh release     # arm64-v8a + x86_64 → android/app/src/main/jniLibs/

# 2. APK (нужен JDK 17):
cd android && ./gradlew assembleDebug
# app/build/outputs/apk/debug/app-debug.apk
```

Переменные: `ANDROID_HOME` (SDK), `ANDROID_NDK_HOME` (по умолчанию
`$ANDROID_HOME/ndk/27.0.12077973`). jniLibs **не коммитятся** (`.gitignore`);
CI (`android.yml`) собирает всё с нуля и прикладывает APK артефактом.

## Тонкости (проверено на эмуляторе)

- **Opus на Android**: `audiopus-sys` не умеет кроссить autotools сам —
  собираем `libopus.a` вручную (`--host`, NDK clang) и отдаём через
  `OPUS_LIB_DIR` + `OPUS_STATIC=1` (см. `build-rust.sh`). Без этого —
  `UnsatisfiedLinkError: cannot locate symbol "opus_encode_float"`.
- **Toolchain**: `cargo` из Homebrew не видит android-таргеты rustup —
  скрипт/CI используют rustup-toolchain (`RUSTC` при необходимости).
- **UI-зависимости** (`eframe/egui/rfd`) — только `cfg(not(target_os))`,
  иначе winit тянет `android-activity` и ломает сборку.
- **JNI**: jni 0.22, канон `EnvUnowned → with_env → resolve` (см. `android_bridge.rs`).
- Проверено на эмуляторе Pixel 5 API 33: установка, запуск, демо-режим,
  диалоги, история чата (включая защищённый контент), навигация.

## Не покрыто (следующие шаги)

- Push-уведомления (FCM), фоновый синк (WorkManager), звонки (нет VoIP-стека).
- EncryptedSharedPreferences для PIN/токена Яндекса (сейчас in-memory).
- CameraX для съёмки сторис (сейчас — системная камера/файлы + музыка из файла).
- Релизная подпись и публикация (сейчас debug-APK).
