# Telecomfort — инструкция разработчика

## 1. Сборка и запуск

```bash
cargo check --bin telecomfort --offline   # быстрая проверка (кэш крейтов)
cargo test --bin telecomfort --offline    # 124 теста
cargo clippy --bin telecomfort --offline  # 0 ошибок — обязательно
cargo fmt                                 # перед коммитом
RUST_LOG=telecomfort=debug cargo run --offline
```

Требования: Rust 1.99+, macOS/Linux. Сеть для cargo **не нужна** (все
зависимости в локальном кэше); новые крейты добавить нельзя — проверяйте
`~/.cargo/registry/src/*/`.

## 2. Как добавить фичу (канонический путь)

Новый метод backend → UI делается **строго в 6 шагов**:

1. **Трейт** (`src/telegram/mod.rs`): сигнатура + DTO-структура
   (derive `Serialize/Deserialize`), дефолт `anyhow::bail!` — чтобы
   старые backend'ы компилировались.
2. **Live** (`src/telegram/backend_grammers.rs`): inherent-метод с raw
   `client.invoke(...)` + однострочная делегация в trait-impl.
   Имена TL-типов сверяйте с
   `target/debug/build/grammers-tl-types-*/out/generated_{types,functions,enums}.rs`.
3. **Mock** (`src/telegram/mock.rs`): детерминированные демо-данные.
4. **App** (`src/ui/app.rs`): поля → метод-действие (`rt.spawn` + `BgEvent`)
   → arm в `pump_events` → хелперы/тесты рядом.
5. **UI** (`src/ui/views/*.rs`): кнопка → метод; окно — по паттерну
   существующих (`egui::Window::new(...).collapsible(false)`).
6. **Тесты**: продлить `mock_management_contract_15` + юнит-тест логики;
   `cargo test/clippy/fmt`; README-строка в «Что добавлено».

Запрещено: сетевые вызовы из UI-потока, `unwrap/expect` на пользовательских
путях, новые зависимости без проверки кэша.

## 3. Ключевые паттерны кода

- **Фоновая задача**: `let tg = Arc::clone(&self.tg); let tx = self.tx.clone();`
  `self.rt.spawn(async move { ... tx.send(BgEvent::...) })`.
- **Опасные действия**: взвод (`gift_armed`, `pw_armed`) или `ConfirmAction`
  + arm в `do_confirm_action` + текст окна.
- **Peer→Input**: `let pref = self.resolve_ref(chat_id).await?;`
  `(&pref).into()` → `InputPeer`; `tl::enums::InputChannel::from(pref)`.
  Access_hash пользователя — через `GetPeerDialogs` (см. `dialog_user`).
- **Upload → InputMedia**: `upload_file` → `uploaded.raw` (+атрибуты);
  существующий документ — `InputMediaDocument` по id/hash/file_reference.
- **Скачивание с прогрессом**: `media::mtproto::download_parallel`
  (чанки, resume, `on_progress`).
- **Крипто**: `password_hash_ph2` — копия формулы grammers_crypto; векторы
  проверять независимым Python (`hashlib`), хардкодить в тест.
- **Атрибут `#[async_trait]`**: при вставке `impl GrammersBackend`
  перед trait-impl не разрывать пару `#[async_trait] + impl ... for ...`.

## 4. Контракт-тесты мока

`mock_management_contract_01…15` в `src/telegram/mock.rs` — executable
спецификация: каждый метод трейта обязан вернуть детерминированный ответ.
Новый метод → новые asserts в `contract_15` (или новый `contract_16` при
переполнении). Демо-режим (`MockTelegram`) позволяет разрабатывать UI
без ключей и сети.

## 5. Конфиг и хранилище (для отладки)

- `config.toml` + `session_*.session` — в `config_dir/telecomfort`;
- кэши — `cache_dir/telecomfort/{stories,avatars,audio,stickers}`;
- SQLite — дедуп/кэши; temp — репосты, `voice_*.wav`, probe-файлы.
- Сброс: удалить `config.toml` (дефолты) или session-файл (релогин).

## 6. Релизный чеклист

1. `cargo test` — зелёный (124).
2. `cargo clippy` — 0 ошибок (warning-и — только pre-existing).
3. `cargo fmt` + `git diff` — только нужное.
4. README: строка фичи + при необходимости USER_GUIDE/ARCHITECTURE.
5. Ветка `feature/*` → MR → merge в `main` (см. историю: паритет-13…17).
