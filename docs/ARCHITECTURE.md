# Telecomfort — архитектура

Диаграммы: [C4 PlantUML](architecture-c4.puml) · [Mermaid](architecture-mermaid.md).
Инструкции: [пользователю](USER_GUIDE.md) · [разработчику](DEVELOPER_GUIDE.md).

## 1. Что это

Нативный десктопный Telegram-клиент на Rust: `eframe/egui` (GUI) + `grammers`
(MTProto) + `tokio` (фон) + `rusqlite` (локальные данные). Один бинарь,
старт в демо-режиме без ключей (mock-backend), live-режим — по API id/hash
с my.telegram.org.

## 2. Принципы

1. **Граница по трейту.** Весь UI зависит только от
   `telegram::TelegramService` (`Arc<dyn>`, ~110 методов). Две реализации:
   `GrammersBackend` (live) и `MockTelegram` (демо + контракт-тесты). Новый
   метод = пункт трейта с дефолтом → реализация в backend → мок → UI.
2. **Фон через BgEvent-pump.** UI-поток никогда не блокируется на сети:
   методы `TelecomfortApp` спавнят задачи в `tokio::runtime::Handle`, те
   шлют `BgEvent` по `mpsc`, `pump_events()` применяет их к состоянию.
3. **Профили изолированы.** Каждый профиль — свой `session_<имя>.session`
   (auth-ключи grammers), свои API-ключи и прокси. Переключение — через
   `PENDING/SWAP_BACKEND` + `swap_backend`.
4. **Офлайн-устойчивость.** Собственные декодеры AV1/H.264/HEVC без asm,
   WAV-райтер и ресемплер свои, PBKDF2/SHA — чистый Rust. Внешний ffmpeg —
   только опциональный фолбэк для постеров.
5. **Опасное — с подтверждением.** Пароль 2FA (двойной клик + проверка
   через GetPasswordSettings), Stars-подарки (взвод + списание звёзд),
   жалоба на спам (ConfirmAction), удаление (ConfirmAction).

## 3. Каталог модулей

| Путь | Роль |
|---|---|
| `src/main.rs` | tracing, tokio-rt, eframe-окно, иконка в коде |
| `src/config.rs` | `AppConfig`: профили, тема, PIN-sha256, автоблок, прокси, правила; `session_path_for` |
| `src/db.rs` | `Db`: дедуп sha256, phash-индекс, ONNX-кэш, загрузки, метки yandex/calibre |
| `src/sync.rs` | `SyncEngine`, десктоп-`notify`, фоновый тикер |
| `src/telegram/mod.rs` | Трейт `TelegramService` + все DTO (`MessageInfo`, `ChatFullInfo`, …) |
| `src/telegram/backend_grammers.rs` | Live: raw `invoke`, upload/download, SRP/PH2, waveform, gifts, stats-парсинг |
| `src/telegram/mock.rs` | Демо-данные + `mock_management_contract_NN` тесты |
| `src/ui/app.rs` | `TelecomfortApp`: состояние, pump, ~150 методов-действий, хелперы (`qr_image_232`, `export_history_html`, `parse_jump_date`) |
| `src/ui/views/` | `chats/contacts/downloads/login/settings/stories` + модалки |
| `src/media/mtproto.rs` | Параллельные чанки, resume, `on_progress` |
| `src/media/{av1,h264,hevc,video}.rs` | Декодеры и `first_frame_builtin` для превью |
| `src/media/voice_rec.rs` | cpal-запись 48кГц моно → ресемпл → WAV 16-bit |
| `src/ai/` | heuristic → CLIP/ONNX → blur; phash+bktree дедуп; visual-кластеры |
| `src/integrations/` | `yandex` (OAuth streaming) + `calibre` (импорт книг) |

## 4. Потоки (кратко, подробно — Mermaid §3–7)

- **Вход**: код / пароль SRP / QR (`exportLoginToken` → QR 232px → опрос
  `importLoginToken` 2.5с). Подсказка пароля — с сервера (`getPassword.hint`).
- **Отправка**: очередь `pending_attach` + подпись → текст / файл / альбом;
  войс (waveform 63×5бит), кружок (`round_message`), стикер/GIF по
  `InputDocument`, отложенная и тихая отправка.
- **Экспорт**: `history_before` страницами по 500 (до 20000) → JSON / HTML;
  опционально медиа (до 150 файлов, относительные ссылки).
- **Синк**: тикер → диалоги/сторис → правила автозагрузки → дедуп →
  yandex/calibre → notify.
- **Защита**: PIN (SHA-256, экран-гейт, автоблок по неактивности), SOCKS5/MTProxy
  (+кнопка проверки отдельным соединением), смена пароля 2FA с верификацией.

## 5. Хранилище

- `config_dir/telecomfort/`: `config.toml`, `session_<профиль>.session`.
- `cache_dir/telecomfort/`: `stories/`, `avatars/`, `audio/`, `stickers/`,
  resume-`.part` файлы.
- SQLite: дедуп-хэши, phash, ONNX-кэш, загрузки, yandex/calibre-метки.
- `temp/telecomfort/`: репосты, `voice_*.wav`, proxy-probe, файлы экспорта.

## 6. Тестирование

- Юнит: QR-хелперы, даты прыжка, HTML-экспорт, PH2 (вектор из Python/hashlib),
  ресемпл/WAV-roundtrip, автоблок-логика, sha256.
- Контракты мока `mock_management_contract_01…15`: каждый метод трейта обязан
  иметь детерминированный мок-ответ; UI пишется против мока в демо-режиме.
- Live-проверки (требуют аккаунт): вход, отправки, 2FA, подарки — вручную.

## 7. Заведомые ограничения (почему так)

- **Войсы — WAV, не Opus**: opus-энкодера нет в офлайн-кэше крейтов; код
  структурирован под будущий `concentus` (`VoiceTake` уже 48кГц моно).
- **Секретные чаты (E2E)**: сознательно не реализованы — самописный E2E без
  аудита опаснее отсутствия; см. README «Невозможное».
- **MTProto-прокси**: реализован локальным SOCKS5-мостом (`src/telegram/mtproxy.rs`):
  obfuscated2 + intermediate/padded-intermediate, без форка grammers.
- **Отправка подарков — только обычные Stars**: коллекционные gift_id требуют
  отдельного API-сегмента (аукционы/ресейл).
