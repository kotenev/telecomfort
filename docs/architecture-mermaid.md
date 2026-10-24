# Telecomfort — диаграммы Mermaid

Рендерятся GitHub, GitLab, VS Code (Markdown Preview Mermaid), Obsidian, kroki.io.
Исходная C4-модель: `architecture-c4.puml`. Нарратив: `ARCHITECTURE.md`.

## 1. Системный контекст

```mermaid
flowchart LR
    U([Пользователь]) -->|GUI eframe/egui| APP[Telecomfort<br/>десктоп-клиент]
    APP -->|MTProto TCP + opt SOCKS5/MTProxy<br/>grammers 0.11| TG[(Telegram MTProto API<br/>DC 1-5)]
    APP -->|HTTPS OAuth<br/>streaming upload| YD[(Yandex Disk)]
    APP -->|файлы книг| CAL[(Calibre<br/>локальная библиотека)]
    APP -->|запись WAV 48кГц<br/>воспроизведение| OS([ОС: микрофон/аудио<br/>cpal / rodio])
```

## 2. Контейнеры процесса и потоки данных

```mermaid
flowchart TB
    subgraph PROC [Процесс telecomfort — 1 бинарь]
        UI[UI-поток<br/>eframe event-loop<br/>пump_events + отрисовка]
        RT[Tokio multi-thread runtime<br/>сетевые задачи]
        UI <-->|mpsc BgEvent<br/>spawn / pump| RT
    end
    RT <-->|invoke / upload / download| TG[(MTProto)]
    RT <-->|SQL| DB[(rusqlite)]
    RT <-->|auth-ключи| SES[[session_*.session]]
    RT <-->|медиа/постеры/resume| CACHE[[cache_dir]]
    UI <-->|load/save| CFG[[config.toml]]
    RT -->|streaming| YD[(Yandex Disk)]
```

## 3. Вход: три способа (sequence)

```mermaid
sequenceDiagram
    participant U as Пользователь
    participant V as login.rs view
    participant A as TelecomfortApp
    participant B as GrammersBackend
    participant S as Telegram

    alt По коду
        U->>V: телефон → Получить код
        V->>A: start_login → profiles_to_cfg
        A->>B: connect_with_proxy
        B->>S: auth.sendCode
        U->>V: код из Telegram
        V->>A: submit_code → sign_in
        A->>V: swap_backend → BgEvent SignedIn
    else По паролю 2FA
        U->>V: пароль (+хинт с сервера)
        V->>A: submit_password → check_password SRP
        A->>V: SignedIn
    else По QR
        U->>V: 📷 Войти по QR
        V->>A: start_qr_login
        A->>B: qr_login_start → token
        B->>V: BgEvent QrToken → QR 232px
        U->>S: скан в телефоне
        loop каждые 2.5с
            B->>S: qr_login_poll → importLoginToken
        end
        B->>V: swap_backend + SignedIn
    end
    V->>A: refresh_dialogs/stories/me + load_stars
```

## 4. Отправка сообщения (sequence)

```mermaid
sequenceDiagram
    participant U as Пользователь
    participant C as chats.rs composer
    participant A as TelecomfortApp
    participant T as TelegramService (Arc dyn)
    participant S as Telegram

    U->>C: 📎 файлы → очередь pending_attach
    U->>C: Ctrl+Enter
    C->>A: send_current — снять очередь
    alt только текст
        A->>T: send_text
    else 1 файл + подпись
        A->>T: send_file → upload → sendMessage
    else N файлов + подпись
        A->>T: send_album → N×upload → sendMultiMedia
    end
    alt 🎙 запись
        U->>C: 🎙 старт → cpal поток
        U->>C: ■ стоп → WAV → send_file
    else 🎤 файл как войс
        A->>T: send_voice — probe rodio → waveform 63×5бит → SendMedia voice
    else ◉ кружок
        A->>T: send_round_video — dims из first_frame → round_message
    else 😀/🎞
        A->>T: send_document_ref — InputDocument по id/hash/ref
    end
    T->>S: updates
    S-->>A: BgEvent Log + ReloadHistory
```

## 5. Экспорт истории с медиа (flowchart)

```mermaid
flowchart TB
    H[⤵ в шапке чата] --> W{Окно экспорта:<br/>формат + медиа?}
    W -->|JSON| J[save dialog]
    W -->|HTML| D[save dialog]
    J --> P[цикл history_before по 500<br/>до 20000]
    D --> P
    P --> M{with_media и HTML?}
    M -->|да| DL[download_message_media ×до 150<br/>в *_files/]
    M -->|нет| R[рендер]
    DL --> R[export_history_html<br/>ссылки или плейсхолдеры]
    R --> WR[запись файла]
    WR --> E[BgEvent HistoryExported]
```

## 6. Смена пароля 2FA (sequence)

```mermaid
sequenceDiagram
    participant U as Пользователь
    participant V as settings.rs
    participant A as TelecomfortApp
    participant B as GrammersBackend
    participant S as Telegram

    U->>V: текущий + новый ×2 + хинт + email
    U->>V: Сменить пароль → взвод
    U->>V: Подтвердить!
    V->>A: change_password
    A->>B: GetPassword → current_algo + srp
    B->>B: calculate_2fa — SRP-проверка
    B->>B: ph2(new) — SHA256/PBKDF2/SHA256
    B->>S: UpdatePasswordSettings
    B->>S: GetPassword заново
    B->>S: GetPasswordSettings новым паролем
    S-->>A: Ok = доказательство корректности
```

## 7. Фоновый синк (flowchart)

```mermaid
flowchart TB
    T[SyncTick каждые N сек] --> G{включён sync?}
    G -->|нет| X[стоп]
    G -->|да| D[refresh_dialogs]
    D --> ST[refresh_stories + превью]
    ST --> R{правила автозагрузки}
    R -->|совпало| DW[download_parallel → dedup sha256 → phash]
    DW --> Y{yandex_auto?}
    Y -->|да| YU[streaming upload]
    Y -->|нет| CL{книга?}
    CL -->|да, calibre_auto| CI[импорт в Calibre]
    CL -->|нет| ND[notify при флагах]
    YU --> ND
```

## 8. Состояния: экраны, вход, блокировка (state)

```mermaid
stateDiagram-v2
    [*] --> Stories: старт (демо/mock)
    Stories --> Chats: 💬
    Chats --> Settings: ⚙
    Settings --> Chats: назад
    Chats --> Contacts: 👥
    Contacts --> Chats: назад
    Chats --> Downloads: ⬇
    Downloads --> Chats: назад

    state Login {
        [*] --> api: ключи/профиль
        api --> connecting: connect
        connecting --> code: код отправлен
        connecting --> done: уже авторизован
        code --> done: код/пароль/QR принят
        code --> password: нужен 2FA
        password --> done: SRP ок
    }

    state Lock {
        [*] --> unlocked: PIN пуст
        [*] --> locked: PIN задан
        unlocked --> locked: задать PIN / замкнуть
        locked --> unlocked: верный PIN
        unlocked --> locked: автоблок по таймауту
    }
```

## 9. Карта хранилища (flowchart)

```mermaid
flowchart LR
    subgraph CONF [config_dir/telecomfort]
        CT[config.toml<br/>профили, тема, PIN-sha256,<br/>прокси, правила]
        S1[session_main.session]
        S2[session_*.session]
    end
    subgraph CACHE [cache_dir/telecomfort]
        ST[stories/ — медиа + постеры]
        AV[avatars/]
        AU[audio/]
        SK[stickers/ — recent + gif]
    end
    subgraph DBF [db]
        SQ[(telecomfort.sqlite<br/>дедуп, phash, onnx-кэш,<br/>загрузки, метки)]
    end
    APP[Telecomfort] --- CONF
    APP --- CACHE
    APP --- DBF
    TMP([temp/telecomfort<br/>repost, voice_*.wav,<br/>proxy-probe, экспорт]) --- APP
```
