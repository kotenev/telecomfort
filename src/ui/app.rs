//! Главный UI Telecomfort (eframe/egui).
//!
//! Экраны: Чаты · Сторис (анти-реклама) · Загрузки · Настройки.
//! Асинхронный Telegram-мост: фоновые tokio-задачи шлют [`BgEvent`] в UI-поток.

use crate::ai::{score_story_full, AdScore, HeuristicAdDetector};
use crate::config::{AppConfig, TelegramSettings};
use crate::db::Db;
use crate::media::AutoDownloader;
use crate::telegram::{AccountInfo, ChatInfo, MessageInfo, MockTelegram, StoryInfo, TelegramService};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum BgEvent {
    Dialogs(Vec<ChatInfo>),
    History { chat_id: i64, items: Vec<MessageInfo> },
    Stories(Vec<StoryInfo>),
    StoryPreview { peer_id: i64, story_id: i32, path: PathBuf },
    /// Готовый постер первого кадра видеосторис.
    StoryPoster { peer_id: i64, story_id: i32, poster: PathBuf },
    /// Прогресс заливки на Яндекс Диск (стриминг чанками).
    UploadProgress { local_path: String, done: u64, total: u64 },
    /// Прогресс скачивания из MTProto (параллельные чанки + resume).
    MediaProgress { label: String, done: u64, total: u64 },
    /// Своё отправленное сообщение (серверный id) — добавить в историю.
    SentMessage { chat_id: i64, item: crate::telegram::MessageInfo },
    /// Своя правка применена — обновить текст локально.
    EditedMessage { chat_id: i64, message_id: i32, text: String },
    /// Удаление применено — выкинуть локально.
    DeletedMessages { chat_id: i64, message_ids: Vec<i32> },
    /// Перезагрузить историю чата (после реакций/чужих действий).
    ReloadHistory { chat_id: i64 },
    /// Аватар скачался — запомнить файл, сбросить текстуру-идентикон.
    AvatarReady { chat_id: i64, path: PathBuf },
    /// Закреплённое сообщение чата (None — нет закрепа).
    PinnedLoaded { chat_id: i64, item: Option<crate::telegram::MessageInfo> },
    /// Папки диалогов сервера.
    FoldersLoaded(Vec<crate::telegram::DialogFilterInfo>),
    /// Контакты телефонной книги.
    ContactsLoaded(Vec<crate::telegram::ContactInfo>),
    /// Перезагрузить список диалогов (после мьют/архив).
    ReloadDialogs,
    /// Аудио скачалось в кэш — начать воспроизведение.
    AudioReady { chat_id: i64, message_id: i32, path: PathBuf },
    /// Пересылка выполнена.
    Forwarded { from_chat: i64, to_chat: i64, count: usize },
    /// Стикер скачался в кэш — показать превью в пузыре.
    StickerReady { message_id: i32, path: PathBuf },
    /// Активные сессии загружены.
    SessionsLoaded(Vec<crate::telegram::SessionInfo>),
    /// Профиль загружен.
    ProfileLoaded(crate::telegram::ProfileInfo),
    /// Сторис опубликована.
    StoryPublished { story_id: i32 },
    /// Топики форума чата.
    TopicsLoaded { chat_id: i64, topics: Vec<crate::telegram::TopicInfo> },
    /// История топика (заменяет общую историю в UI).
    TopicHistoryLoaded { chat_id: i64, topic_id: i32, items: Vec<crate::telegram::MessageInfo> },
    /// Создан новый топик.
    TopicCreated { chat_id: i64, topic_id: i32 },
    /// Результаты глобального поиска.
    GlobalSearchResults(Vec<(i64, crate::telegram::MessageInfo)>),
    /// Запланированные сообщения чата.
    ScheduledLoaded { chat_id: i64, items: Vec<crate::telegram::MessageInfo> },
    /// Инвайт-ссылка готова.
    InviteLinkReady(String),
    /// Видео-превью скачалось в кэш.
    VideoThumbReady { message_id: i32, path: PathBuf },
    /// Фото веб-превью скачалось в кэш.
    WebThumbReady { photo_id: i64, path: PathBuf },
    /// Канал/группа созданы.
    ChannelCreated { chat_id: i64 },
    /// Сторис удалены — перезагрузить ленту.
    StoriesDeleted,
    /// Опрос создан.
    PollCreated { chat_id: i64, message_id: i32 },
    /// Геопозиция отправлена.
    GeoSent { chat_id: i64, message_id: i32 },
    /// Ответ на сторис отправлен.
    StoryReplySent { story_id: i32, message_id: i32 },
    /// Просмотры сторис загружены.
    StoryViewsLoaded { story_id: i32, views: i32 },
    /// Стелс-режим включён.
    StealthEnabled,
    /// Зрители сторис загружены.
    StoryViewersLoaded { story_id: i32, count: i32, viewers: Vec<String> },
    /// Медиа-таб истории загружен.
    MediaTabLoaded { chat_id: i64, items: Vec<crate::telegram::MessageInfo> },
    /// Чёрный список загружен.
    BlockedLoaded(Vec<crate::telegram::BlockedInfo>),
    /// Контакт расшарен.
    ShareContactSent { chat_id: i64, message_id: i32 },
    /// Опрос остановлен.
    PollStopped { chat_id: i64 },
    /// Все закрепы загружены.
    PinnedListLoaded { chat_id: i64, items: Vec<crate::telegram::MessageInfo> },
    /// Инвайт отозван.
    InviteRevoked,
    /// Участники загружены.
    ParticipantsLoaded { chat_id: i64, items: Vec<crate::telegram::ParticipantInfo> },
    /// Подпись сторис обновлена.
    StoryCaptionSaved { story_id: i32 },
    /// Перезагрузить ленту сторис.
    ReloadStories,
    Me(AccountInfo),
    Log(String),
    Error(String),
    Connected { backend_desc: String },
    CodeSent,
    SignedIn,
    SyncTick,
    SyncDone { stories: usize, downloads: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Chats,
    Stories,
    Downloads,
    Contacts,
    Settings,
}

pub struct StoryCard {
    pub info: StoryInfo,
    pub score: AdScore,
    pub revealed: bool,
    pub texture: Option<egui::TextureHandle>,
    pub texture_blurred: Option<egui::TextureHandle>,
    /// Локальный кадр (реальный PNG/JPG из кэша или сгенерированный mock).
    pub local_path: Option<PathBuf>,
    pub preview_loading: bool,
    pub visual_done: bool,
}

/// Активное воспроизведение голосового/аудио (rodio 0.23).
/// `_sink` держим живым — без него звук останавливается.
pub struct AudioPlayback {
    pub chat_id: i64,
    pub message_id: i32,
    pub path: PathBuf,
    _sink: rodio::MixerDeviceSink,
    player: rodio::Player,
}

/// Публикация сторис: выбранный файл + подпись (окно подтверждения).
pub struct StoryPublish {
    pub path: PathBuf,
    pub caption: String,
}

/// Опасное действие, требующее подтверждения окном.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Очистить историю чата (revoke — и у всех).
    ClearHistory { chat_id: i64, revoke: bool },
    /// Покинуть канал/супергруппу.
    LeaveChannel { chat_id: i64 },
    /// Удалить свои сторис.
    DeleteStories { story_ids: Vec<i32> },
}

pub struct TelecomfortApp {
    pub cfg: AppConfig,
    pub db: Arc<Db>,
    pub tg: Arc<dyn TelegramService>,
    pub rt: tokio::runtime::Handle,
    pub tx: mpsc::Sender<BgEvent>,
    pub rx: Arc<Mutex<mpsc::Receiver<BgEvent>>>,

    pub screen: Screen,
    pub dialogs: Vec<ChatInfo>,
    pub selected_chat: Option<i64>,
    pub history: HashMap<i64, Vec<MessageInfo>>,
    pub stories: Vec<StoryCard>,
    pub logs: Vec<String>,
    pub status: String,
    pub last_sync: String,
    pub me_label: String,

    // Настройки (редактируемые поля)
    pub ad_threshold: f32,
    pub blur_sigma: f32,
    pub blur_only_regions: bool,
    pub auto_hide_ads: bool,
    pub download_dir: String,
    pub yandex_token: String,
    pub yandex_folder: String,
    pub yandex_auto: bool,
    pub calibre_lib: String,
    pub calibre_auto: bool,
    pub calibre_delete: bool,

    // Фоновая синхронизация
    pub sync_enabled: bool,
    pub sync_interval: String,
    pub sync_auto_new: bool,
    pub notify_dl: bool,
    pub notify_ad: bool,
    sync_abort: Option<tokio::task::AbortHandle>,

    // Новое правило автозагрузки (форма)
    pub new_rule_chat: String,
    pub new_rule_photo: bool,
    pub new_rule_video: bool,
    pub new_rule_audio: bool,
    pub new_rule_doc: bool,
    pub new_rule_max: String,

    // Загрузки: фильтры + прогресс заливок
    pub dl_query: String,
    pub dl_filter: String,
    pub dl_cluster_threshold: u32,
    pub uploads: HashMap<String, (u64, u64)>,
    /// Кэш миниатюр для групп похожих (путь → текстура 120px).
    /// Чистится implicitly: ключ — путь файла, stale-превью приемлемы.
    pub dl_thumbs: HashMap<String, egui::TextureHandle>,

    // Чаты: композер и пузыри (паритет с официальным клиентом)
    /// Черновики ввода по чатам (сохраняются при переключении диалогов).
    pub composer: HashMap<i64, String>,
    /// Текущий текст ввода (синкается с composer при select_chat).
    pub composer_current: String,
    /// Поиск по загруженной истории текущего чата.
    pub chat_search: String,
    /// Ответ: (chat_id, msg_id, сниппет цитаты).
    pub reply_to: Option<(i64, i32, String)>,
    /// Режим правки: (chat_id, msg_id) — текст уже подставлен в композер.
    pub editing: Option<(i64, i32)>,
    /// Кэш текстур аватаров (chat_id → текстура 64px) + файлы + запрошенные.
    pub avatars: HashMap<i64, egui::TextureHandle>,
    pub avatar_files: HashMap<i64, PathBuf>,
    avatar_requested: HashSet<i64>,

    // Второй parity-пакет: пины/папки/контакты/пересылка/голосовые
    /// Закреплённые сообщения по чатам (подгружаются при открытии).
    pub pinned: HashMap<i64, crate::telegram::MessageInfo>,
    /// Папки диалогов сервера (вкладки-фильтры) + выбранная (None = все).
    pub folders: Vec<crate::telegram::DialogFilterInfo>,
    pub folder_sel: Option<i32>,
    /// Контакты телефонной книги + флаг загрузки.
    pub contacts: Vec<crate::telegram::ContactInfo>,
    pub contacts_loaded: bool,
    /// Выбор цели пересылки: (from_chat, msg_ids).
    pub forward_pick: Option<(i64, Vec<i32>)>,
    /// Активное воспроизведение голосового/аудио.
    pub playback: Option<AudioPlayback>,

    // Третий parity-пакет: отложка/стикеры/сессии/профиль/публикация сторис
    /// Окно отложенной отправки: часы задержки (текст композера подставляется).
    pub sched_open: bool,
    pub sched_hours: String,
    /// Превью стикеров в пузырях (msg_id → текстура) + файлы + запрошенные.
    pub sticker_tex: HashMap<i32, egui::TextureHandle>,
    pub sticker_files: HashMap<i32, PathBuf>,
    sticker_requested: HashSet<i32>,
    /// Активные сессии + флаг загрузки.
    pub sessions: Vec<crate::telegram::SessionInfo>,
    pub sessions_loaded: bool,
    /// Профиль: редактируемые поля + флаг загрузки.
    pub profile_first: String,
    pub profile_last: String,
    pub profile_about: String,
    pub profile_loaded: bool,
    /// Публикация сторис: выбранный файл + подпись + окно.
    pub story_publish: Option<StoryPublish>,
    /// ID своего аккаунта (для «мои сторис»).
    pub me_id: Option<i64>,

    // Четвёртый parity-пакет: топики/поиск/превью/модерация/создание
    /// Топики форума текущего чата + выбранный (None = общий чат).
    pub topics: Vec<crate::telegram::TopicInfo>,
    pub topic_sel: Option<i32>,
    /// Создание топика: окно + название.
    pub topic_create_open: bool,
    pub topic_create_title: String,
    /// Глобальный поиск: запрос + результаты (chat_id, сообщение).
    pub gsearch: String,
    pub gsearch_results: Vec<(i64, crate::telegram::MessageInfo)>,
    /// Видео-превью в пузырях (msg_id → текстура) + файлы + запрошенные.
    pub video_thumbs: HashMap<i32, egui::TextureHandle>,
    pub video_thumb_files: HashMap<i32, PathBuf>,
    video_thumb_requested: HashSet<i32>,
    /// Веб-превью: фото (photo_id → текстура) + файлы + запрошенные.
    pub web_thumbs: HashMap<i64, egui::TextureHandle>,
    pub web_thumb_files: HashMap<i64, PathBuf>,
    web_thumb_requested: HashSet<i64>,
    /// Создание группы/канала: окно + поля.
    pub create_chat_open: bool,
    pub create_chat_title: String,
    pub create_chat_about: String,
    pub create_chat_broadcast: bool,
    /// Инвайт-ссылка: текст + окно.
    pub invite_link: Option<String>,
    /// Подтверждения опасных действий: очистка/выход/удаление сторис.
    pub confirm_action: Option<ConfirmAction>,
    /// Запланированные сообщения текущего чата + окно.
    pub scheduled_open: bool,
    pub scheduled: Vec<crate::telegram::MessageInfo>,
    /// Добавление контакта: окно + поля.
    pub contact_add_open: bool,
    pub contact_add_phone: String,
    pub contact_add_first: String,
    pub contact_add_last: String,

    // Пятый parity-пакет: опросы/гео/сторис-интерактив
    /// Конструктор опроса: окно + поля.
    pub poll_open: bool,
    pub poll_question: String,
    pub poll_options: String,
    pub poll_quiz: bool,
    pub poll_correct: String,
    pub poll_anonymous: bool,
    pub poll_multiple: bool,
    /// Геопозиция: окно + поля.
    pub geo_open: bool,
    pub geo_lat: String,
    pub geo_long: String,
    /// Ответ на сторис: окно + текст + цель.
    pub story_reply_open: bool,
    pub story_reply_text: String,
    pub story_reply_target: Option<(i64, i32)>,
    /// Просмотры своих сторис (story_id → views).
    pub story_views: HashMap<i32, i32>,

    // Шестой parity-пакет: сторис-интерактив, медиапоиск, профиль, контакты
    /// Зрители сторис (story_id → (count, имена)) + окно.
    pub story_viewers: HashMap<i32, (i32, Vec<String>)>,
    pub story_viewers_open: Option<i32>,
    /// Медиа-табы истории: None = всё, Some("photo"|"video"|"link").
    pub media_tab: Option<String>,
    /// Username + загрузка аватара в настройках.
    pub profile_username: String,
    /// Чёрный список + флаг загрузки.
    pub blocked: Vec<crate::telegram::BlockedInfo>,
    pub blocked_loaded: bool,

    // Седьмой parity-пакет: шаринг контактов, опросы-менеджмент, модерация
    /// Выбор контакта для шаринга: целевой чат.
    pub contact_share_pick: Option<i64>,
    /// Все закрепы чата + окно.
    pub pinned_more: HashMap<i64, Vec<crate::telegram::MessageInfo>>,
    pub pinned_more_open: Option<i64>,
    /// Участники чата + окно.
    pub participants: HashMap<i64, Vec<crate::telegram::ParticipantInfo>>,
    pub participants_open: Option<i64>,
    /// Редактирование подписи своей сторис.
    pub story_edit_open: Option<i32>,
    pub story_edit_text: String,

    // ИИ: CLIP-лайт и ONNX
    pub clip_enabled: bool,
    pub onnx_enabled: bool,
    pub onnx_model: String,
    pub onnx_input: String,
    pub onnx_provider: String,
    pub ffmpeg_status: String,

    // Профили/Логин
    pub profiles: Vec<TelegramSettings>,
    pub active_profile: String,
    pub api_id: String,
    pub api_hash: String,
    pub phone: String,
    pub code: String,
    pub password: String,
    pub login_stage: String,
    pub new_profile_name: String,

    preview_requested: HashSet<(i64, i32)>,
    detector: HeuristicAdDetector,
}

impl TelecomfortApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        mut cfg: AppConfig,
        db: Arc<Db>,
        rt: tokio::runtime::Handle,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        cfg.ensure_profile();
        let active = cfg.active_profile.clone();
        let cur = cfg
            .profiles
            .iter()
            .find(|p| p.name == active)
            .cloned()
            .or_else(|| cfg.profiles.first().cloned())
            .unwrap();
        let mut app = Self {
            ad_threshold: cfg.stories.ad_threshold,
            blur_sigma: cfg.stories.blur_sigma,
            blur_only_regions: cfg.stories.blur_only_ad_regions,
            auto_hide_ads: cfg.stories.auto_hide_ads,
            download_dir: cfg.downloader.download_dir.to_string_lossy().to_string(),
            yandex_token: cfg.yandex.oauth_token.clone(),
            yandex_folder: cfg.yandex.remote_folder.clone(),
            yandex_auto: cfg.yandex.auto_upload,
            calibre_lib: cfg.calibre.library_path.clone(),
            calibre_auto: cfg.calibre.auto_import_books,
            calibre_delete: cfg.calibre.delete_after_import,
            sync_enabled: cfg.sync.enabled,
            sync_interval: cfg.sync.interval_secs.to_string(),
            sync_auto_new: cfg.sync.auto_download_new,
            notify_dl: cfg.sync.notify_on_download,
            notify_ad: cfg.sync.notify_on_ad,
            sync_abort: None,
            new_rule_chat: "*".into(),
            new_rule_photo: true,
            new_rule_video: true,
            new_rule_audio: true,
            new_rule_doc: true,
            new_rule_max: "1500".into(),
            dl_query: String::new(),
            dl_filter: "Все".into(),
            dl_cluster_threshold: cfg.downloader.cluster_max_dist.min(10),
            dl_thumbs: HashMap::new(),
            composer: HashMap::new(),
            composer_current: String::new(),
            chat_search: String::new(),
            reply_to: None,
            editing: None,
            avatars: HashMap::new(),
            avatar_files: HashMap::new(),
            avatar_requested: HashSet::new(),
            pinned: HashMap::new(),
            folders: Vec::new(),
            folder_sel: None,
            contacts: Vec::new(),
            contacts_loaded: false,
            forward_pick: None,
            playback: None,
            sched_open: false,
            sched_hours: "1".into(),
            sticker_tex: HashMap::new(),
            sticker_files: HashMap::new(),
            sticker_requested: HashSet::new(),
            sessions: Vec::new(),
            sessions_loaded: false,
            profile_first: String::new(),
            profile_last: String::new(),
            profile_about: String::new(),
            profile_loaded: false,
            story_publish: None,
            me_id: None,
            topics: Vec::new(),
            topic_sel: None,
            topic_create_open: false,
            topic_create_title: String::new(),
            gsearch: String::new(),
            gsearch_results: Vec::new(),
            video_thumbs: HashMap::new(),
            video_thumb_files: HashMap::new(),
            video_thumb_requested: HashSet::new(),
            web_thumbs: HashMap::new(),
            web_thumb_files: HashMap::new(),
            web_thumb_requested: HashSet::new(),
            create_chat_open: false,
            create_chat_title: String::new(),
            create_chat_about: String::new(),
            create_chat_broadcast: false,
            invite_link: None,
            confirm_action: None,
            scheduled_open: false,
            scheduled: Vec::new(),
            contact_add_open: false,
            contact_add_phone: String::new(),
            contact_add_first: String::new(),
            contact_add_last: String::new(),
            poll_open: false,
            poll_question: String::new(),
            poll_options: String::new(),
            poll_quiz: false,
            poll_correct: String::new(),
            poll_anonymous: true,
            poll_multiple: false,
            geo_open: false,
            geo_lat: String::new(),
            geo_long: String::new(),
            story_reply_open: false,
            story_reply_text: String::new(),
            story_reply_target: None,
            story_views: HashMap::new(),
            story_viewers: HashMap::new(),
            story_viewers_open: None,
            media_tab: None,
            profile_username: String::new(),
            blocked: Vec::new(),
            blocked_loaded: false,
            contact_share_pick: None,
            pinned_more: HashMap::new(),
            pinned_more_open: None,
            participants: HashMap::new(),
            participants_open: None,
            story_edit_open: None,
            story_edit_text: String::new(),
            uploads: HashMap::new(),
            clip_enabled: cfg.stories.clip_enabled,
            onnx_enabled: cfg.stories.onnx_enabled,
            onnx_model: cfg.stories.onnx_model_path.clone(),
            onnx_input: cfg.stories.onnx_input_name.clone(),
            onnx_provider: crate::ai::normalize_provider(&cfg.stories.onnx_provider).to_string(),
            ffmpeg_status: if crate::media::video::ffmpeg_available() {
                "встроенные декодеры H.264/HEVC/AV1 ✓ + ffmpeg найден — видео-превью из реальных кадров".into()
            } else {
                "встроенные декодеры H.264/HEVC/AV1 ✓ (ffmpeg не найден — редкие кодеки могут дать плейсхолдер)".into()
            },
            profiles: cfg.profiles.clone(),
            active_profile: cfg.active_profile.clone(),
            api_id: if cur.api_id == 0 {
                String::new()
            } else {
                cur.api_id.to_string()
            },
            api_hash: cur.api_hash.clone(),
            phone: cur.phone.clone(),
            cfg,
            db,
            tg: Arc::new(MockTelegram),
            rt,
            tx,
            rx: Arc::new(Mutex::new(rx)),
            screen: Screen::Stories,
            dialogs: vec![],
            selected_chat: None,
            history: HashMap::new(),
            stories: vec![],
            logs: vec![],
            status: "демо-режим (mock). Введите API-ключи в Настройках и войдите.".into(),
            last_sync: "ещё не было".into(),
            me_label: "демо (mock)".into(),
            code: String::new(),
            password: String::new(),
            login_stage: "api".into(),
            new_profile_name: String::new(),
            preview_requested: HashSet::new(),
            detector: HeuristicAdDetector::new(),
        };
        app.refresh_dialogs();
        app.refresh_stories();
        app.refresh_me();
        app.restart_background_sync();
        app
    }

    pub fn log(&mut self, s: impl Into<String>) {
        let s = s.into();
        tracing::info!("{s}");
        self.logs.push(s);
        if self.logs.len() > 400 {
            self.logs.remove(0);
        }
    }

    #[allow(dead_code)]
    pub fn sync_cfg_to_app(&mut self) {
        self.sync_enabled = self.cfg.sync.enabled;
        self.sync_interval = self.cfg.sync.interval_secs.to_string();
        self.sync_auto_new = self.cfg.sync.auto_download_new;
        self.notify_dl = self.cfg.sync.notify_on_download;
        self.notify_ad = self.cfg.sync.notify_on_ad;
    }

    fn pump_events(&mut self) {
        if let Some(swapped) = super::views::login::take_swap_backend() {
            self.tg = swapped;
            self.preview_requested.clear();
            self.log("переключено на live-бэкенд grammers");
        }
        // Окончание воспроизведения — чистим состояние плеера.
        if self.playback.as_ref().map(|p| p.player.empty()).unwrap_or(false) {
            self.playback = None;
        }
        let events: Vec<BgEvent> = {
            let rx = self.rx.lock().unwrap();
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        for e in events {
            match e {
                BgEvent::Dialogs(d) => {
                    self.dialogs = d;
                    self.log(format!("диалогов: {}", self.dialogs.len()));
                }
                BgEvent::History { chat_id, items } => {
                    if let Some(max_id) = items.iter().map(|m| m.id).max() {
                        // watermark для фонового синка обновляется только фоном,
                        // но ручное открытие чата тоже двигает его вперёд (без автокачки).
                        let known = self.db.get_last_message_id(chat_id);
                        if max_id > known && known != 0 {
                            // не перезаписываем здесь, чтобы фон докачал новое
                        } else if known == 0 {
                            self.db.set_last_message_id(chat_id, max_id);
                        }
                    }
                    self.history.insert(chat_id, items);
                }
                BgEvent::SentMessage { chat_id, item } => {
                    self.log(format!("отправлено (msg {})", item.id));
                    self.history.entry(chat_id).or_default().push(item);
                }
                BgEvent::EditedMessage { chat_id, message_id, text } => {
                    if let Some(items) = self.history.get_mut(&chat_id) {
                        if let Some(m) = items.iter_mut().find(|m| m.id == message_id) {
                            m.text = text;
                            m.edited = true;
                        }
                    }
                    self.log(format!("правка применена (msg {message_id})"));
                }
                BgEvent::DeletedMessages { chat_id, message_ids } => {
                    if let Some(items) = self.history.get_mut(&chat_id) {
                        items.retain(|m| !message_ids.contains(&m.id));
                    }
                    self.log(format!("удалено сообщений: {}", message_ids.len()));
                }
                BgEvent::ReloadHistory { chat_id } => {
                    self.load_history(chat_id);
                }
                BgEvent::AvatarReady { chat_id, path } => {
                    self.avatar_files.insert(chat_id, path);
                    // Сбрасываем идентикон — следующий кадр подхватит фото.
                    self.avatars.remove(&chat_id);
                }
                BgEvent::PinnedLoaded { chat_id, item } => {
                    match item {
                        Some(m) => {
                            self.pinned.insert(chat_id, m);
                        }
                        None => {
                            self.pinned.remove(&chat_id);
                        }
                    }
                }
                BgEvent::FoldersLoaded(folders) => {
                    self.folders = folders;
                    self.log(format!("папок диалогов: {}", self.folders.len()));
                }
                BgEvent::ContactsLoaded(contacts) => {
                    self.contacts = contacts;
                    self.contacts_loaded = true;
                    self.log(format!("контактов: {}", self.contacts.len()));
                }
                BgEvent::ReloadDialogs => {
                    self.refresh_dialogs();
                }
                BgEvent::AudioReady { chat_id, message_id, path } => {
                    self.start_playback(chat_id, message_id, path);
                }
                BgEvent::Forwarded { from_chat, to_chat, count } => {
                    self.forward_pick = None;
                    self.log(format!("переслано сообщений: {count} (чат {from_chat} → {to_chat}"));
                }
                BgEvent::StickerReady { message_id, path } => {
                    self.sticker_files.insert(message_id, path);
                    // Сбрасываем плейсхолдер — следующий кадр подхватит превью.
                    self.sticker_tex.remove(&message_id);
                }
                BgEvent::SessionsLoaded(sessions) => {
                    self.sessions = sessions;
                    self.sessions_loaded = true;
                    self.log(format!("сессий: {}", self.sessions.len()));
                }
                BgEvent::ProfileLoaded(p) => {
                    self.profile_first = p.first_name;
                    self.profile_last = p.last_name;
                    self.profile_about = p.about;
                    self.profile_username = p.username.unwrap_or_default();
                    self.profile_loaded = true;
                }
                BgEvent::StoryPublished { story_id } => {
                    self.story_publish = None;
                    self.log(format!("сторис опубликована (id {story_id})"));
                    self.refresh_stories();
                }
                BgEvent::TopicsLoaded { chat_id, topics } => {
                    if self.selected_chat == Some(chat_id) {
                        self.topics = topics;
                    }
                }
                BgEvent::TopicHistoryLoaded { chat_id, topic_id, items } => {
                    if self.selected_chat == Some(chat_id) {
                        self.topic_sel = Some(topic_id);
                        self.history.insert(chat_id, items);
                    }
                }
                BgEvent::TopicCreated { chat_id, topic_id } => {
                    self.topic_create_open = false;
                    self.topic_create_title.clear();
                    self.log(format!("топик создан (id {topic_id})"));
                    self.load_topics(chat_id);
                }
                BgEvent::GlobalSearchResults(results) => {
                    self.gsearch_results = results;
                    self.log(format!("глобальный поиск: {}", self.gsearch_results.len()));
                }
                BgEvent::ScheduledLoaded { chat_id, items } => {
                    if self.selected_chat == Some(chat_id) {
                        self.scheduled = items;
                        self.scheduled_open = true;
                    }
                }
                BgEvent::InviteLinkReady(link) => {
                    self.invite_link = Some(link);
                }
                BgEvent::VideoThumbReady { message_id, path } => {
                    self.video_thumb_files.insert(message_id, path);
                    self.video_thumbs.remove(&message_id);
                }
                BgEvent::WebThumbReady { photo_id, path } => {
                    self.web_thumb_files.insert(photo_id, path);
                    self.web_thumbs.remove(&photo_id);
                }
                BgEvent::ChannelCreated { chat_id } => {
                    self.create_chat_open = false;
                    self.create_chat_title.clear();
                    self.create_chat_about.clear();
                    self.log(format!("чат создан (id {chat_id})"));
                    self.refresh_dialogs();
                }
                BgEvent::StoriesDeleted => {
                    self.refresh_stories();
                }
                BgEvent::PollCreated { chat_id, message_id } => {
                    self.poll_open = false;
                    self.poll_question.clear();
                    self.poll_options.clear();
                    self.poll_correct.clear();
                    self.log(format!("опрос создан (msg {message_id})"));
                    self.load_history(chat_id);
                }
                BgEvent::GeoSent { chat_id, message_id } => {
                    self.geo_open = false;
                    self.log(format!("геопозиция отправлена (msg {message_id})"));
                    self.load_history(chat_id);
                }
                BgEvent::StoryReplySent { story_id, message_id } => {
                    self.story_reply_open = false;
                    self.story_reply_text.clear();
                    self.story_reply_target = None;
                    self.log(format!("ответ на сторис {story_id} отправлен (msg {message_id})"));
                }
                BgEvent::StoryViewsLoaded { story_id, views } => {
                    self.story_views.insert(story_id, views);
                }
                BgEvent::StealthEnabled => {
                    self.log("🥷 стелс-режим сторис включён".to_string());
                }
                BgEvent::StoryViewersLoaded { story_id, count, viewers } => {
                    self.story_views.insert(story_id, count);
                    self.story_viewers.insert(story_id, (count, viewers));
                    self.story_viewers_open = Some(story_id);
                }
                BgEvent::MediaTabLoaded { chat_id, items } => {
                    if self.selected_chat == Some(chat_id) {
                        self.history.insert(chat_id, items);
                    }
                }
                BgEvent::BlockedLoaded(blocked) => {
                    self.blocked = blocked;
                    self.blocked_loaded = true;
                    // Сбрасываем флаг, если список пуст и это был первый запрос.
                }
                BgEvent::ShareContactSent { chat_id, message_id } => {
                    self.contact_share_pick = None;
                    self.log(format!("контакт отправлен (msg {message_id})"));
                    self.load_history(chat_id);
                }
                BgEvent::PollStopped { chat_id } => {
                    self.log("опрос остановлен".to_string());
                    self.load_history(chat_id);
                }
                BgEvent::PinnedListLoaded { chat_id, items } => {
                    self.pinned_more.insert(chat_id, items);
                    self.pinned_more_open = Some(chat_id);
                }
                BgEvent::InviteRevoked => {
                    self.invite_link = None;
                    self.log("инвайт-ссылка отозвана".to_string());
                }
                BgEvent::ParticipantsLoaded { chat_id, items } => {
                    self.participants.insert(chat_id, items);
                    self.participants_open = Some(chat_id);
                }
                BgEvent::ReloadStories => {
                    self.refresh_stories();
                }
                BgEvent::StoryCaptionSaved { story_id } => {
                    self.story_edit_open = None;
                    self.story_edit_text.clear();
                    self.log(format!("подпись сторис {story_id} обновлена"));
                    self.refresh_stories();
                }
                BgEvent::Stories(stories) => {
                    self.preview_requested.retain(|(pid, sid)| {
                        !stories.iter().any(|s| s.peer_id == *pid && s.story_id == *sid)
                    });
                    // Фаза 1: превью + базовый скоринг (текст + визуал + CLIP-лайт).
                    struct Pending {
                        info: StoryInfo,
                        local: Option<PathBuf>,
                        img: Option<image::DynamicImage>,
                        score: AdScore,
                    }
                    let mut pending: Vec<Pending> = Vec::with_capacity(stories.len());
                    for info in stories {
                        // Восстанавливаем кэшированное превью из SQLite, если есть.
                        let cached = self
                            .db
                            .story_preview(info.peer_id, info.story_id)
                            .map(PathBuf::from)
                            .filter(|p| p.exists());
                        let local = info
                            .preview_path
                            .clone()
                            .map(PathBuf::from)
                            .filter(|p| p.exists())
                            .or(cached);
                        // Постер видео подменяем кадром, если он уже извлечён.
                        let img: Option<image::DynamicImage> =
                            local.as_ref().and_then(|p| display_image_for(p));
                        let score = crate::ai::score_story_base(
                            &self.detector,
                            &info.caption,
                            info.has_url,
                            img.as_ref(),
                            self.clip_enabled,
                        );
                        pending.push(Pending { info, local, img, score });
                    }
                    // Фаза 2: streaming ONNX — одна сессия на все кадры пачки
                    // + persistent-кэш SQLite (повторы не пересчитываются).
                    let img_refs: Vec<Option<&image::DynamicImage>> =
                        pending.iter().map(|p| p.img.as_ref()).collect();
                    let boosts = crate::ai::try_onnx_boost_many(
                        Some(self.db.as_ref()),
                        &self.onnx_model,
                        self.onnx_enabled,
                        &img_refs,
                        &self.onnx_input,
                        &self.onnx_provider,
                    );
                    // Фаза 3: применяем бусты, сохраняем решения, строим карты.
                    let mut cards = Vec::with_capacity(pending.len());
                    for (mut p, boost) in pending.into_iter().zip(boosts) {
                        if let Some((b, reasons)) = boost {
                            p.score.apply_clip_boost(b, reasons);
                        }
                        let visual_done = p.img.is_some();
                        let key = format!("{}:{}", p.info.peer_id, p.info.story_id);
                        self.db.save_ad_decision(
                            &key,
                            p.score.score,
                            &p.score.reasons.join("; "),
                            p.score.is_ad(self.ad_threshold) && self.auto_hide_ads,
                        );
                        cards.push(StoryCard {
                            info: p.info,
                            score: p.score,
                            revealed: false,
                            texture: None,
                            texture_blurred: None,
                            local_path: p.local,
                            preview_loading: false,
                            visual_done,
                        });
                    }
                    // Сортируем: сначала реклама (чтобы анти-реклама была видна).
                    cards.sort_by(|a, b| {
                        b.score
                            .score
                            .partial_cmp(&a.score.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    self.stories = cards;
                    self.log(format!("сторис: {}", self.stories.len()));
                    self.request_story_previews();
                    // Уведомление о новой рекламе (опционально).
                    if self.notify_ad {
                        let ads = self
                            .stories
                            .iter()
                            .filter(|c| c.score.is_ad(self.ad_threshold))
                            .count();
                        if ads > 0 {
                            crate::sync::notify(
                                "Telecomfort: сторис",
                                &format!("найдено рекламы: {ads}"),
                            );
                        }
                    }
                }
                BgEvent::StoryPreview { peer_id, story_id, path } => {
                    // Если это видео — сразу запускаем извлечение постера.
                    let is_video = crate::media::video::is_video_path(&path);
                    if is_video {
                        self.spawn_poster_extraction(peer_id, story_id, path.clone());
                    }
                    if let Some(card) = self
                        .stories
                        .iter_mut()
                        .find(|c| c.info.peer_id == peer_id && c.info.story_id == story_id)
                    {
                        card.local_path = Some(path.clone());
                        card.preview_loading = false;
                        card.texture = None;
                        card.texture_blurred = None;
                        if !card.visual_done {
                            // Для видео скоринг уточнится по постеру (StoryPoster).
                            let img_opt: Option<image::DynamicImage> = if is_video {
                                None
                            } else {
                                image::open(&path).ok()
                            };
                            if let Some(img) = img_opt.as_ref().or_else(|| {
                                // Видео без постера: скорим по заглушке, честно.
                                None
                            }) {
                                card.score = score_story_full(
                                    &self.detector,
                                    &card.info.caption,
                                    card.info.has_url,
                                    Some(img),
                                    self.clip_enabled,
                                    &self.onnx_model,
                                    self.onnx_enabled,
                                    &self.onnx_input,
                                    &self.onnx_provider,
                                    Some(self.db.as_ref()),
                                );
                            } else if is_video {
                                card.score = score_story_full(
                                    &self.detector,
                                    &card.info.caption,
                                    card.info.has_url,
                                    None,
                                    self.clip_enabled,
                                    &self.onnx_model,
                                    self.onnx_enabled,
                                    &self.onnx_input,
                                    &self.onnx_provider,
                                    Some(self.db.as_ref()),
                                );
                            }
                            persist_card_score(
                                &self.db,
                                peer_id,
                                story_id,
                                &card.info.caption,
                                &path,
                                &card.score,
                                self.ad_threshold,
                            );
                            card.visual_done = !is_video;
                        }
                    }
                }
                BgEvent::StoryPoster { peer_id, story_id, poster } => {
                    // Считаем скор без удержания &mut на stories (иначе E0499 с self.log).
                    let img_opt = image::open(&poster).ok();
                    if let Some(img) = img_opt {
                        let new_score = {
                            // Временный immutable borrow для caption/has_url.
                            let card_ref = match self
                                .stories
                                .iter()
                                .find(|c| c.info.peer_id == peer_id && c.info.story_id == story_id)
                            {
                                Some(c) => (c.info.caption.clone(), c.info.has_url),
                                None => (String::new(), false),
                            };
                            score_story_full(
                                &self.detector,
                                &card_ref.0,
                                card_ref.1,
                                Some(&img),
                                self.clip_enabled,
                                &self.onnx_model,
                                self.onnx_enabled,
                                &self.onnx_input,
                                &self.onnx_provider,
                                Some(self.db.as_ref()),
                            )
                        };
                        if let Some(card) = self
                            .stories
                            .iter_mut()
                            .find(|c| c.info.peer_id == peer_id && c.info.story_id == story_id)
                        {
                            card.texture = None;
                            card.texture_blurred = None;
                            card.score = new_score;
                            persist_card_score(
                                &self.db,
                                peer_id,
                                story_id,
                                &card.info.caption.clone(),
                                &poster,
                                &card.score,
                                self.ad_threshold,
                            );
                            card.visual_done = true;
                        }
                        self.log(format!(
                            "видеосторис {peer_id}:{story_id} — кадр извлечён, скоринг уточнён"
                        ));
                    } else if let Some(card) = self
                        .stories
                        .iter_mut()
                        .find(|c| c.info.peer_id == peer_id && c.info.story_id == story_id)
                    {
                        card.visual_done = true;
                    }
                }
                BgEvent::UploadProgress { local_path, done, total } => {
                    self.uploads.insert(local_path, (done, total));
                }
                BgEvent::MediaProgress { label, done, total } => {
                    self.uploads.insert(label, (done, total));
                }
                BgEvent::Me(info) => {
                    self.me_label = if info.name.is_empty() {
                        format!("id {}", info.id)
                    } else {
                        info.name.clone()
                    };
                    self.me_id = Some(info.id);
                }
                BgEvent::Log(s) => self.log(s),
                BgEvent::Error(s) => {
                    self.status = s.clone();
                    self.log(format!("ошибка: {s}"));
                }
                BgEvent::Connected { backend_desc } => {
                    self.status = format!("подключено: {backend_desc}");
                    self.log(self.status.clone());
                    self.preview_requested.clear();
                    self.refresh_dialogs();
                    self.refresh_stories();
                    self.refresh_me();
                }
                BgEvent::CodeSent => {
                    self.login_stage = "code".into();
                    self.status = "код отправлен, введите код из Telegram".into();
                }
                BgEvent::SignedIn => {
                    self.login_stage = "done".into();
                    self.status = "вход выполнен".into();
                    self.preview_requested.clear();
                    self.refresh_dialogs();
                    self.refresh_stories();
                    self.refresh_me();
                }
                BgEvent::SyncTick => {
                    self.last_sync = chrono::Local::now().format("%H:%M:%S").to_string();
                    self.run_background_once();
                }
                BgEvent::SyncDone { stories, downloads } => {
                    self.log(format!("фон: сторис {stories}, новых файлов {downloads}"));
                }
            }
        }
    }

    /// Запросить скачивание превью для карточек без локального файла (по одному разу).
    pub fn request_story_previews(&mut self) {
        let cache_dir = crate::telegram::stories_cache_dir();
        let _ = std::fs::create_dir_all(&cache_dir);
        for card in &mut self.stories {
            if card.local_path.is_some() || card.preview_loading {
                continue;
            }
            let key = (card.info.peer_id, card.info.story_id);
            if self.preview_requested.contains(&key) {
                continue;
            }
            self.preview_requested.insert(key);
            card.preview_loading = true;
            let tg = Arc::clone(&self.tg);
            let tx = self.tx.clone();
            let (peer_id, story_id) = key;
            let dir = cache_dir.clone();
            self.rt.spawn(async move {
                match tg.download_story_media(peer_id, story_id, &dir).await {
                    Ok(path) => {
                        // Для видео сразу извлекаем постер первого кадра.
                        if crate::media::video::is_video_path(&path) {
                            let poster = crate::media::video::poster_path_for(&path);
                            if crate::media::video::poster_fresh(&path, &poster) {
                                let _ = tx.send(BgEvent::StoryPreview { peer_id, story_id, path: path.clone() });
                                let _ = tx.send(BgEvent::StoryPoster { peer_id, story_id, poster });
                            } else {
                                let _ = tx.send(BgEvent::StoryPreview { peer_id, story_id, path: path.clone() });
                                match crate::media::video::extract_poster(&path, &poster).await {
                                    Ok(p) => {
                                        let _ = tx.send(BgEvent::StoryPoster { peer_id, story_id, poster: p });
                                    }
                                    Err(e) => {
                                        tracing::debug!("poster {peer_id}:{story_id}: {e:#}");
                                    }
                                }
                            }
                        } else {
                            let _ = tx.send(BgEvent::StoryPreview { peer_id, story_id, path });
                        }
                    }
                    Err(e) => {
                        tracing::debug!("story preview {peer_id}:{story_id} failed: {e:#}");
                        let _ = tx.send(BgEvent::Log(format!(
                            "превью сторис {peer_id}:{story_id} недоступно: {e}"
                        )));
                    }
                }
            });
        }
    }

    /// Извлечь постер для уже скачанного видео (повторный вызов из StoryPreview).
    /// Работает и без ffmpeg — через встроенные декодеры H.264/HEVC/AV1 (чистый Rust).
    pub fn spawn_poster_extraction(&mut self, peer_id: i64, story_id: i32, video: PathBuf) {
        let poster = crate::media::video::poster_path_for(&video);
        if crate::media::video::poster_fresh(&video, &poster) {
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let _ = tx.send(BgEvent::StoryPoster { peer_id, story_id, poster });
            });
            return;
        }
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match crate::media::video::extract_poster(&video, &poster).await {
                Ok(p) => {
                    let _ = tx.send(BgEvent::StoryPoster { peer_id, story_id, poster: p });
                }
                Err(e) => {
                    tracing::debug!("poster {peer_id}:{story_id}: {e:#}");
                }
            }
        });
    }

    pub fn refresh_dialogs(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.dialogs().await {
                Ok(d) => {
                    let _ = tx.send(BgEvent::Dialogs(d));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("dialogs: {e:#}")));
                }
            }
        });
        // Папки — отдельным запросом, не блокируем список диалогов.
        self.load_folders();
    }

    pub fn refresh_me(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Ok(me) = tg.me().await {
                let _ = tx.send(BgEvent::Me(me));
            }
        });
    }

    pub fn refresh_stories(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.stories().await {
                Ok(s) => {
                    let _ = tx.send(BgEvent::Stories(s));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("stories: {e:#}")));
                }
            }
        });
    }

    pub fn load_history(&mut self, chat_id: i64) {
        self.selected_chat = Some(chat_id);
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.history(chat_id, 100).await {
                Ok(items) => {
                    let _ = tx.send(BgEvent::History { chat_id, items });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("history: {e:#}")));
                }
            }
        });
    }

    /// Выбрать чат: сохранить черновик, подставить черновик нового,
    /// сбросить reply/edit/поиск, загрузить историю, отметить прочитанным.
    pub fn select_chat(&mut self, chat_id: i64) {
        if let Some(old) = self.selected_chat {
            if old != chat_id {
                let draft = std::mem::take(&mut self.composer_current);
                self.composer.insert(old, draft.clone());
                // Черновик и на сервер (синк между устройствами), best-effort.
                let tg = Arc::clone(&self.tg);
                self.rt.spawn(async move {
                    let _ = tg.save_draft(old, &draft).await;
                });
            }
        }
        self.selected_chat = Some(chat_id);
        self.composer_current = self.composer.get(&chat_id).cloned().unwrap_or_default();
        self.reply_to = None;
        self.editing = None;
        self.chat_search.clear();
        self.media_tab = None;
        self.topics.clear();
        self.topic_sel = None;
        self.load_history(chat_id);
        self.load_pinned(chat_id);
        self.load_topics(chat_id);
        let tg = Arc::clone(&self.tg);
        self.rt.spawn(async move {
            let _ = tg.mark_read(chat_id).await;
        });
    }

    /// Отправить текущее (или сохранить правку). Пустое — игнор.
    /// После успешной отправки черновик стирается и локально, и на сервере.
    pub fn send_current(&mut self) {
        let Some(chat_id) = self.selected_chat else {
            return;
        };
        let text = self.composer_current.trim().to_string();
        if text.is_empty() {
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        if let Some((ec, em)) = self.editing {
            // Режим правки: текст уже в композере.
            self.editing = None;
            self.composer_current.clear();
            self.composer.insert(chat_id, String::new());
            self.rt.spawn(async move {
                match tg.edit_text(ec, em, &text).await {
                    Ok(()) => {
                        let _ = tx.send(BgEvent::EditedMessage {
                            chat_id: ec,
                            message_id: em,
                            text,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(BgEvent::Error(format!("правка: {e:#}")));
                    }
                }
            });
            return;
        }
        let reply = self.reply_to.clone().filter(|(c, _, _)| *c == chat_id).map(|(_, m, _)| m);
        self.reply_to = None;
        self.composer_current.clear();
        self.composer.insert(chat_id, String::new());
        self.rt.spawn(async move {
            match tg.send_text(chat_id, &text, reply).await {
                Ok(sent_id) => {
                    let item = crate::telegram::MessageInfo {
                        id: sent_id,
                        chat_id,
                        text,
                        date: chrono::Local::now().timestamp(),
                        has_media: false,
                        media_kind: None,
                        file_name: None,
                        file_size: None,
                        outgoing: true,
                        sender: None,
                        edited: false,
                        reactions: None,
                        views: None,
                        forwards: None,
                        poll: None,
                        geo: None,
                        contact: None,
                        web_preview: None,
                        topic_id: None,
                    };
                    let _ = tx.send(BgEvent::SentMessage { chat_id, item });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("отправка: {e:#}")));
                }
            }
        });
    }

    /// Начать ответ на сообщение (подставляет баннер над композером).
    pub fn start_reply(&mut self, chat_id: i64, msg: &crate::telegram::MessageInfo) {
        let snippet = if msg.text.is_empty() {
            msg.file_name.clone().unwrap_or_else(|| format!("медиа #{}", msg.id))
        } else {
            crate::utils::truncate(&msg.text, 80)
        };
        self.editing = None;
        self.reply_to = Some((chat_id, msg.id, snippet));
    }

    /// Начать правку своего сообщения (текст — в композер).
    pub fn start_edit(&mut self, chat_id: i64, msg: &crate::telegram::MessageInfo) {
        self.reply_to = None;
        self.editing = Some((chat_id, msg.id));
        self.composer_current = msg.text.clone();
        self.composer.insert(chat_id, msg.text.clone());
    }

    /// Удалить сообщение (revoke — у всех).
    pub fn delete_msg(&mut self, chat_id: i64, message_id: i32) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.delete_messages(chat_id, &[message_id]).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::DeletedMessages {
                        chat_id,
                        message_ids: vec![message_id],
                    });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("удаление: {e:#}")));
                }
            }
        });
    }

    /// Поставить реакцию (повтор той же сервером убирается — toggle).
    pub fn react_to(&mut self, chat_id: i64, message_id: i32, emoji: &str) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let emoji = emoji.to_string();
        self.rt.spawn(async move {
            match tg.react(chat_id, message_id, &emoji).await {
                Ok(()) => {
                    // Счётчики — только с сервера: перезагружаем историю.
                    let _ = tx.send(BgEvent::ReloadHistory { chat_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("реакция: {e:#}")));
                }
            }
        });
    }

    /// Прикрепить файл: выбор через нативный диалог → заливка → отправка
    /// (фото — как фото, остальное — документом; подпись — текст композера).
    pub fn attach_file(&mut self) {
        let Some(chat_id) = self.selected_chat else {
            self.log("сначала выберите чат");
            return;
        };
        let Some(path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        let caption = self.composer_current.trim().to_string();
        self.composer_current.clear();
        self.composer.insert(chat_id, String::new());
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.log(format!("заливка файла: {}…", path.display()));
        self.rt.spawn(async move {
            match tg.send_file(chat_id, &path, &caption).await {
                Ok(sent_id) => {
                    let item = crate::telegram::MessageInfo {
                        id: sent_id,
                        chat_id,
                        text: caption,
                        date: chrono::Local::now().timestamp(),
                        has_media: true,
                        media_kind: Some("document".into()),
                        file_name: path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string()),
                        file_size: std::fs::metadata(&path).ok().map(|m| m.len() as i64),
                        outgoing: true,
                        sender: None,
                        edited: false,
                        reactions: None,
                        views: None,
                        forwards: None,
                        poll: None,
                        geo: None,
                        contact: None,
                        web_preview: None,
                        topic_id: None,
                    };
                    let _ = tx.send(BgEvent::SentMessage { chat_id, item });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("отправка файла: {e:#}")));
                }
            }
        });
    }

    /// Аватар чата 64px: кэш → файл → запрос скачивания → идентикон.
    /// Идентикон возвращается сразу (и как fallback без фото).
    pub fn ensure_avatar(
        &mut self,
        ctx: &egui::Context,
        chat_id: i64,
        title: &str,
    ) -> egui::TextureHandle {
        if let Some(t) = self.avatars.get(&chat_id) {
            return t.clone();
        }
        if let Some(p) = self.avatar_files.get(&chat_id).cloned() {
            if let Ok(img) = image::open(&p) {
                let thumb = img.resize(64, 64, image::imageops::FilterType::Triangle);
                let tex = ctx.load_texture(
                    format!("avatar::{chat_id}"),
                    to_egui(&thumb),
                    egui::TextureOptions::default(),
                );
                self.avatars.insert(chat_id, tex.clone());
                return tex;
            }
        }
        if self.avatar_requested.insert(chat_id) {
            let tg = Arc::clone(&self.tg);
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Ok(Some(path)) = tg.avatar_path(chat_id).await {
                    let _ = tx.send(BgEvent::AvatarReady { chat_id, path });
                }
            });
        }
        let tex = ctx.load_texture(
            format!("avatar::{chat_id}"),
            identicon(title, chat_id),
            egui::TextureOptions::default(),
        );
        self.avatars.insert(chat_id, tex.clone());
        tex
    }

    // --- Второй parity-пакет: пины/мьют/архив/папки/опросы/пересылка/голосовые ---

    /// Загрузить закреплённое сообщение чата (баннер под шапкой).
    pub fn load_pinned(&mut self, chat_id: i64) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.pinned_message(chat_id).await {
                Ok(item) => {
                    let _ = tx.send(BgEvent::PinnedLoaded { chat_id, item });
                }
                Err(e) => {
                    tracing::debug!("pinned load failed: {e}");
                }
            }
        });
    }

    /// Закрепить/открепить сообщение (сервер проверяет права).
    pub fn toggle_pin(&mut self, chat_id: i64, message_id: i32, unpin: bool) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = if unpin {
                tg.unpin_message(chat_id, message_id).await
            } else {
                tg.pin_message(chat_id, message_id).await
            };
            match res {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(
                        if unpin { "откреплено".into() } else { "закреплено".into() },
                    ));
                    let _ = tx.send(BgEvent::ReloadHistory { chat_id });
                    match tg.pinned_message(chat_id).await {
                        Ok(item) => {
                            let _ = tx.send(BgEvent::PinnedLoaded { chat_id, item });
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("закреп: {e:#}")));
                }
            }
        });
    }

    /// Заглушить/разглушить чат. Список обновляем через ReloadDialogs.
    pub fn toggle_mute(&mut self, chat_id: i64) {
        let muted = self.dialogs.iter().find(|d| d.id == chat_id).map(|d| d.muted).unwrap_or(false);
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.mute(chat_id, !muted).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(
                        if !muted { "чат заглушён".into() } else { "мут снят".into() },
                    ));
                    let _ = tx.send(BgEvent::ReloadDialogs);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("мьют: {e:#}")));
                }
            }
        });
    }

    /// В архив/из архива. Список обновляем через ReloadDialogs.
    pub fn toggle_archive(&mut self, chat_id: i64) {
        let archived =
            self.dialogs.iter().find(|d| d.id == chat_id).map(|d| d.archived).unwrap_or(false);
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.archive(chat_id, !archived).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(
                        if !archived { "чат в архиве".into() } else { "чат из архива".into() },
                    ));
                    let _ = tx.send(BgEvent::ReloadDialogs);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("архив: {e:#}")));
                }
            }
        });
    }

    /// Загрузить папки диалогов сервера (вкладки-фильтры).
    pub fn load_folders(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.dialog_filters().await {
                Ok(folders) => {
                    let _ = tx.send(BgEvent::FoldersLoaded(folders));
                }
                Err(e) => {
                    tracing::debug!("folders failed: {e}");
                }
            }
        });
    }

    /// Загрузить контакты телефонной книги (экран Контакты).
    pub fn load_contacts(&mut self) {
        if self.contacts_loaded {
            return;
        }
        self.contacts_loaded = true;
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.contacts().await {
                Ok(contacts) => {
                    let _ = tx.send(BgEvent::ContactsLoaded(contacts));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("контакты: {e:#}")));
                }
            }
        });
    }

    /// Начать пересылку: запомнить источник, UI покажет выбор цели.
    pub fn start_forward(&mut self, chat_id: i64, message_id: i32) {
        self.forward_pick = Some((chat_id, vec![message_id]));
        self.log(format!("пересылка msg {message_id}: выберите целевой чат слева"));
    }

    /// Выполнить пересылку в целевой чат.
    pub fn do_forward(&mut self, to_chat: i64) {
        let Some((from_chat, ids)) = self.forward_pick.clone() else {
            return;
        };
        if from_chat == to_chat {
            self.log("пересылка в тот же чат — отменено");
            self.forward_pick = None;
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.forward(from_chat, &ids, to_chat).await {
                Ok(count) => {
                    let _ = tx.send(BgEvent::Forwarded { from_chat, to_chat, count });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("пересылка: {e:#}")));
                }
            }
        });
    }

    /// Проголосовать в опросе (один вариант; multiple-choice — по одному за раз).
    pub fn vote_poll(&mut self, chat_id: i64, message_id: i32, option: Vec<u8>) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.vote(chat_id, message_id, option).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::ReloadHistory { chat_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("голосование: {e:#}")));
                }
            }
        });
    }

    /// Вкл/выкл воспроизведения голосового/аудио (повторный тап — стоп).
    /// Файл докачивается в аудиокэш (не в Загрузки), затем AudioReady.
    pub fn play_audio(&mut self, chat_id: i64, msg: &crate::telegram::MessageInfo) {
        if let Some(pb) = &self.playback {
            if pb.chat_id == chat_id && pb.message_id == msg.id {
                self.playback = None;
                return;
            }
        }
        self.playback = None;
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let fname = msg.file_name.clone().unwrap_or_else(|| format!("audio_{}.ogg", msg.id));
        let ext = std::path::Path::new(&fname)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ogg");
        let dest = crate::telegram::audio_cache_dir()
            .join(format!("{chat_id}_{}.{ext}", msg.id));
        let mid = msg.id;
        self.log(format!("голосовое msg {mid}: загрузка…"));
        self.rt.spawn(async move {
            std::fs::create_dir_all(dest.parent().unwrap()).ok();
            let fresh = std::fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false);
            if !fresh {
                if let Err(e) = tg.download_message_media(chat_id, mid, dest.parent().unwrap()).await
                {
                    let _ = tx.send(BgEvent::Error(format!("аудио: {e:#}")));
                    return;
                }
                // download_message_media кладёт рядом файл со своим именем —
                // находим самый свежий подходящий и переименовываем.
                if !dest.exists() {
                    if let Ok(mut entries) = std::fs::read_dir(dest.parent().unwrap()) {
                        let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p == dest {
                                continue;
                            }
                            if let Ok(meta) = entry.metadata() {
                                if let Ok(mtime) = meta.modified() {
                                    if best.as_ref().map(|(t, _)| *t < mtime).unwrap_or(true) {
                                        best = Some((mtime, p));
                                    }
                                }
                            }
                        }
                        if let Some((_, src)) = best {
                            std::fs::rename(&src, &dest).ok();
                        }
                    }
                }
            }
            if dest.exists() {
                let _ = tx.send(BgEvent::AudioReady { chat_id, message_id: mid, path: dest });
            } else {
                let _ = tx.send(BgEvent::Error("аудио: файл не скачался".into()));
            }
        });
    }

    /// Запустить воспроизведение скачанного файла
    /// (rodio 0.23: `DeviceSinkBuilder` + `play`, CoreAudio/ALSA/WASAPI).
    fn start_playback(&mut self, chat_id: i64, message_id: i32, path: PathBuf) {
        let f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                self.log(format!("плеер: {e:#}"));
                return;
            }
        };
        let mut sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(s) => s,
            Err(e) => {
                self.log(format!("плеер: нет аудио-устройства ({e:?})"));
                return;
            }
        };
        sink.log_on_drop(false);
        match rodio::play(sink.mixer(), f) {
            Ok(player) => {
                self.playback =
                    Some(AudioPlayback { chat_id, message_id, path, _sink: sink, player });
                self.log(format!("▶ воспроизведение msg {message_id}"));
            }
            Err(e) => {
                self.log(format!("плеер: не декодируется ({e:?})"));
            }
        }
    }

    /// Отложенная отправка: текст композера + задержка в часах.
    pub fn send_scheduled(&mut self, hours: f64) {
        let Some(chat_id) = self.selected_chat else {
            return;
        };
        let text = self.composer_current.trim().to_string();
        if text.is_empty() || !(hours > 0.0) {
            return;
        }
        let at = std::time::SystemTime::now()
            + std::time::Duration::from_secs((hours * 3600.0) as u64);
        let reply = self.reply_to.clone().filter(|(c, _, _)| *c == chat_id).map(|(_, m, _)| m);
        self.reply_to = None;
        self.sched_open = false;
        self.composer_current.clear();
        self.composer.insert(chat_id, String::new());
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.send_scheduled(chat_id, &text, reply, at).await {
                Ok(id) => {
                    let _ = tx.send(BgEvent::Log(format!("отложено (msg {id}, через {hours} ч)")));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("отложка: {e:#}")));
                }
            }
        });
    }

    /// Превью стикера в пузыре (128px): кэш → файл → запрос скачивания → плейсхолдер.
    pub fn ensure_sticker(
        &mut self,
        ctx: &egui::Context,
        chat_id: i64,
        msg: &crate::telegram::MessageInfo,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.sticker_tex.get(&msg.id) {
            return Some(t.clone());
        }
        if let Some(p) = self.sticker_files.get(&msg.id).cloned() {
            if let Ok(img) = image::open(&p) {
                let thumb = img.resize(128, 128, image::imageops::FilterType::Triangle);
                let tex = ctx.load_texture(
                    format!("sticker::{}", msg.id),
                    to_egui(&thumb),
                    egui::TextureOptions::default(),
                );
                self.sticker_tex.insert(msg.id, tex.clone());
                return Some(tex);
            }
        }
        if self.sticker_requested.insert(msg.id) {
            let tg = Arc::clone(&self.tg);
            let tx = self.tx.clone();
            let mid = msg.id;
            let dir = crate::telegram::stickers_cache_dir();
            let fname = msg
                .file_name
                .clone()
                .unwrap_or_else(|| format!("sticker_{mid}.webp"));
            self.rt.spawn(async move {
                std::fs::create_dir_all(&dir).ok();
                let dest = dir.join(format!("{chat_id}_{mid}_{fname}"));
                if !dest.exists() {
                    if tg.download_message_media(chat_id, mid, &dir).await.is_err() {
                        return;
                    }
                    // download_message_media кладёт файл со своим именем.
                    if !dest.exists() {
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p == dest {
                                    continue;
                                }
                                if let Ok(meta) = entry.metadata() {
                                    if let Ok(mtime) = meta.modified() {
                                        if best.as_ref().map(|(t, _)| *t < mtime).unwrap_or(true) {
                                            best = Some((mtime, p));
                                        }
                                    }
                                }
                            }
                            if let Some((_, src)) = best {
                                std::fs::rename(&src, &dest).ok();
                            }
                        }
                    }
                }
                if dest.exists() {
                    let _ = tx.send(BgEvent::StickerReady { message_id: mid, path: dest });
                }
            });
        }
        None
    }

    /// Поделиться контактом из телефонной книги в чат.
    pub fn share_contact_to(&mut self, chat_id: i64, c: &crate::telegram::ContactInfo) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let (phone, first, last) = (c.phone.clone(), c.name.clone(), String::new());
        // Имя делим на first/last по первому пробелу (как в официальном клиенте).
        let (first, last) = match first.split_once(' ') {
            Some((f, l)) => (f.to_string(), l.to_string()),
            None => (first, last),
        };
        self.contact_share_pick = None;
        self.rt.spawn(async move {
            match tg.share_contact(chat_id, &phone, &first, &last).await {
                Ok(message_id) => {
                    let _ = tx.send(BgEvent::ShareContactSent { chat_id, message_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("шаринг: {e:#}")));
                }
            }
        });
    }

    /// Остановить свой опрос (закрыть голосование).
    pub fn stop_poll(&mut self, chat_id: i64, message_id: i32) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.stop_poll(chat_id, message_id).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::PollStopped { chat_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("стоп опроса: {e:#}")));
                }
            }
        });
    }

    /// Загрузить все закрепы чата (окно).
    pub fn load_pinned_all(&mut self, chat_id: i64) {
        if self.pinned_more.contains_key(&chat_id) {
            self.pinned_more_open = Some(chat_id);
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.pinned_list(chat_id, 10).await {
                Ok(items) => {
                    let _ = tx.send(BgEvent::PinnedListLoaded { chat_id, items });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("закрепы: {e:#}")));
                }
            }
        });
    }

    /// Отозвать текущую инвайт-ссылку чата.
    pub fn revoke_invite(&mut self, chat_id: i64) {
        let Some(link) = self.invite_link.clone() else {
            return;
        };
        if link == "загрузка…" {
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.revoke_invite(chat_id, &link).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::InviteRevoked);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("отзыв: {e:#}")));
                }
            }
        });
    }

    /// Загрузить участников канала/супергруппы (окно).
    pub fn load_participants(&mut self, chat_id: i64) {
        if self.participants.contains_key(&chat_id) {
            self.participants_open = Some(chat_id);
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.participants(chat_id, 100).await {
                Ok(items) => {
                    let _ = tx.send(BgEvent::ParticipantsLoaded { chat_id, items });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("участники: {e:#}")));
                }
            }
        });
    }

    /// Исключить участника (нужны права админа).
    pub fn kick_participant(&mut self, chat_id: i64, user_id: i64) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.kick_participant(chat_id, user_id).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("участник исключён".into()));
                    match tg.participants(chat_id, 100).await {
                        Ok(items) => {
                            let _ = tx.send(BgEvent::ParticipantsLoaded { chat_id, items });
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("кик: {e:#}")));
                }
            }
        });
    }

    /// Скрыть/показать сторис пользователя в ленте.
    pub fn hide_peer_stories(&mut self, peer_id: i64, hidden: bool) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.hide_peer_stories(peer_id, hidden).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(
                        if hidden { "сторис автора скрыты".into() } else { "сторис автора возвращены".into() },
                    ));
                    let _ = tx.send(BgEvent::ReloadStories);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("скрытие: {e:#}")));
                }
            }
        });
    }

    /// Сохранить подпись своей сторис.
    pub fn save_story_caption(&mut self, story_id: i32) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let caption = self.story_edit_text.trim().to_string();
        self.rt.spawn(async move {
            match tg.edit_story_caption(story_id, &caption).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::StoryCaptionSaved { story_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("подпись: {e:#}")));
                }
            }
        });
    }

    /// Загрузить чёрный список.
    pub fn load_blocked(&mut self) {
        if self.blocked_loaded {
            return;
        }
        self.blocked_loaded = true;
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.blocked_list().await {
                Ok(b) => {
                    let _ = tx.send(BgEvent::BlockedLoaded(b));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("чёрный список: {e:#}")));
                }
            }
        });
    }

    /// Загрузить активные сессии (экран Настройки).
    pub fn load_sessions(&mut self) {
        if self.sessions_loaded {
            return;
        }
        self.sessions_loaded = true;
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.sessions().await {
                Ok(s) => {
                    let _ = tx.send(BgEvent::SessionsLoaded(s));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("сессии: {e:#}")));
                }
            }
        });
    }

    /// Завершить все сессии, кроме текущей.
    pub fn terminate_others(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.terminate_others().await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("другие сессии завершены".into()));
                    match tg.sessions().await {
                        Ok(s) => {
                            let _ = tx.send(BgEvent::SessionsLoaded(s));
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("сессии: {e:#}")));
                }
            }
        });
    }

    /// Загрузить профиль (имя + о себе) в форму настроек.
    pub fn load_profile(&mut self) {
        if self.profile_loaded {
            return;
        }
        self.profile_loaded = true;
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.get_profile().await {
                Ok(p) => {
                    let _ = tx.send(BgEvent::ProfileLoaded(p));
                }
                Err(e) => {
                    tracing::debug!("profile failed: {e}");
                }
            }
        });
    }

    /// Сохранить профиль из формы настроек.
    pub fn save_profile(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let (f, l, a) = (
            self.profile_first.trim().to_string(),
            self.profile_last.trim().to_string(),
            self.profile_about.trim().to_string(),
        );
        self.rt.spawn(async move {
            match tg.update_profile(&f, &l, &a).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("профиль обновлён".into()));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("профиль: {e:#}")));
                }
            }
        });
    }

    /// Выбрать файл сторис (фото/видео) → окно подтверждения с подписью.
    pub fn pick_story(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Медиа", &["jpg", "jpeg", "png", "webp", "mp4", "mov"])
            .pick_file()
        else {
            return;
        };
        self.story_publish = Some(StoryPublish { path, caption: String::new() });
        self.screen = Screen::Stories;
    }

    /// Опубликовать сторис (видно контактам). Фоновая задача, затем refresh.
    pub fn do_publish_story(&mut self) {
        let Some(sp) = self.story_publish.take() else {
            return;
        };
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.log(format!("публикация сторис: {}…", sp.path.display()));
        self.rt.spawn(async move {
            match tg.publish_story(&sp.path, &sp.caption).await {
                Ok(id) => {
                    let _ = tx.send(BgEvent::StoryPublished { story_id: id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("сторис: {e:#}")));
                }
            }
        });
    }

    /// Удалить свои сторис (с подтверждением через confirm_action).
    pub fn delete_stories(&mut self, story_ids: Vec<i32>) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.delete_stories(&story_ids).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("сторис удалены".into()));
                    // Перезагрузка сторис — через refresh_stories вызывающего.
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("удаление сторис: {e:#}")));
                }
            }
        });
    }

    /// Загрузить топики форума чата (пусто — не форум).
    pub fn load_topics(&mut self, chat_id: i64) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.forum_topics(chat_id).await {
                Ok(topics) => {
                    let _ = tx.send(BgEvent::TopicsLoaded { chat_id, topics });
                }
                Err(e) => {
                    tracing::debug!("topics failed: {e}");
                }
            }
        });
    }

    /// Выбрать топик: None — общий чат (обычная история), Some — история топика.
    pub fn select_topic(&mut self, chat_id: i64, topic: Option<i32>) {
        self.topic_sel = topic;
        self.reply_to = None;
        self.editing = None;
        match topic {
            None => self.load_history(chat_id),
            Some(tid) => {
                let tg = Arc::clone(&self.tg);
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    match tg.topic_history(chat_id, tid, 100).await {
                        Ok(items) => {
                            let _ = tx.send(BgEvent::TopicHistoryLoaded {
                                chat_id,
                                topic_id: tid,
                                items,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Error(format!("топик: {e:#}")));
                        }
                    }
                });
            }
        }
    }

    /// Создать топик форума.
    pub fn do_create_topic(&mut self, chat_id: i64) {
        let title = self.topic_create_title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.create_topic(chat_id, &title).await {
                Ok(topic_id) => {
                    let _ = tx.send(BgEvent::TopicCreated { chat_id, topic_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("топик: {e:#}")));
                }
            }
        });
    }

    /// Отправить в текущий топик (если выбран) — иначе обычная отправка.
    pub fn send_to_current_topic(&mut self) -> bool {
        let (Some(chat_id), Some(topic_id)) = (self.selected_chat, self.topic_sel) else {
            return false;
        };
        let text = self.composer_current.trim().to_string();
        if text.is_empty() {
            return false;
        }
        self.composer_current.clear();
        self.composer.insert(chat_id, String::new());
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.send_to_topic(chat_id, topic_id, &text).await {
                Ok(sent_id) => {
                    let item = crate::telegram::MessageInfo {
                        id: sent_id,
                        chat_id,
                        text,
                        date: chrono::Local::now().timestamp(),
                        has_media: false,
                        media_kind: None,
                        file_name: None,
                        file_size: None,
                        outgoing: true,
                        sender: None,
                        edited: false,
                        reactions: None,
                        views: None,
                        forwards: None,
                        poll: None,
                        geo: None,
                        contact: None,
                        web_preview: None,
                        topic_id: Some(topic_id),
                    };
                    let _ = tx.send(BgEvent::SentMessage { chat_id, item });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("топик: {e:#}")));
                }
            }
        });
        true
    }

    /// Глобальный поиск по всем чатам (серверный, лимит 50).
    pub fn do_global_search(&mut self) {
        let q = self.gsearch.trim().to_string();
        if q.is_empty() {
            self.gsearch_results.clear();
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.global_search(&q, 50).await {
                Ok(results) => {
                    let _ = tx.send(BgEvent::GlobalSearchResults(results));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("поиск: {e:#}")));
                }
            }
        });
    }

    /// Загрузить запланированные сообщения чата (окно ⏰).
    pub fn load_scheduled(&mut self, chat_id: i64) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.scheduled_history(chat_id).await {
                Ok(items) => {
                    let _ = tx.send(BgEvent::ScheduledLoaded { chat_id, items });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("запланированные: {e:#}")));
                }
            }
        });
    }

    /// Создать опрос из конструктора (вопрос + строки вариантов).
    pub fn do_create_poll(&mut self, chat_id: i64) {
        let question = self.poll_question.trim().to_string();
        let options: Vec<String> = self
            .poll_options
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if question.is_empty() || options.len() < 2 {
            self.log("опрос: нужны вопрос и ≥2 вариантов".to_string());
            return;
        }
        let correct = if self.poll_quiz {
            self.poll_correct.trim().parse::<usize>().ok().filter(|&i| i < options.len())
        } else {
            None
        };
        if self.poll_quiz && correct.is_none() {
            self.log("викторина: укажите номер правильного варианта (с 0)".to_string());
            return;
        }
        let (quiz, anon, mult) = (self.poll_quiz, self.poll_anonymous, self.poll_multiple);
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.create_poll(chat_id, &question, &options, quiz, correct, anon, mult).await {
                Ok(message_id) => {
                    let _ = tx.send(BgEvent::PollCreated { chat_id, message_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("опрос: {e:#}")));
                }
            }
        });
    }

    /// Отправить геопозицию (точку) в текущий чат.
    pub fn do_send_geo(&mut self, chat_id: i64) {
        let parse = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok();
        let (lat, long) = (parse(&self.geo_lat), parse(&self.geo_long));
        let (Some(lat), Some(long)) = (lat, long) else {
            self.log("гео: введите широту и долготу числами".to_string());
            return;
        };
        self.geo_open = false;
        self.geo_lat.clear();
        self.geo_long.clear();
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.send_geo(chat_id, lat, long).await {
                Ok(message_id) => {
                    let _ = tx.send(BgEvent::GeoSent { chat_id, message_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("гео: {e:#}")));
                }
            }
        });
    }

    /// Ответить на сторис текстом (ответ уходит в чат владельца).
    pub fn do_story_reply(&mut self) {
        let (Some((peer_id, story_id)), text) =
            (self.story_reply_target, self.story_reply_text.trim().to_string())
        else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.story_reply(peer_id, story_id, &text).await {
                Ok(message_id) => {
                    let _ = tx.send(BgEvent::StoryReplySent { story_id, message_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("ответ на сторис: {e:#}")));
                }
            }
        });
    }

    /// Загрузить просмотры своей сторис (👁 на карточке).
    pub fn load_story_views(&mut self, story_id: i32) {
        if self.story_views.contains_key(&story_id) {
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.story_views(story_id).await {
                Ok(views) => {
                    let _ = tx.send(BgEvent::StoryViewsLoaded { story_id, views });
                }
                Err(e) => {
                    tracing::debug!("story views failed: {e}");
                }
            }
        });
    }

    /// Включить стелс-режим сторис (прошлые+будущие просмотры скрыты).
    pub fn enable_stealth(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.enable_stealth().await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::StealthEnabled);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("стелс: {e:#}")));
                }
            }
        });
    }

    /// Реакция на чужую сторис (повтор — toggle на сервере).
    pub fn story_react(&mut self, peer_id: i64, story_id: i32, emoji: &str) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let emoji = emoji.to_string();
        self.rt.spawn(async move {
            if let Err(e) = tg.story_react(peer_id, story_id, &emoji).await {
                let _ = tx.send(BgEvent::Error(format!("реакция на сторис: {e:#}")));
            }
        });
    }

    /// Закрепить/открепить свою сторис в профиле.
    pub fn toggle_story_pin(&mut self, story_id: i32, pinned: bool) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.toggle_story_pin(story_id, !pinned).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(
                        if !pinned { "сторис закреплена".into() } else { "сторис откреплена".into() },
                    ));
                    let _ = tx.send(BgEvent::StoriesDeleted);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("закреп сторис: {e:#}")));
                }
            }
        });
    }

    /// Загрузить зрителей своей сторис (первые 20 имён + окно).
    pub fn load_story_viewers(&mut self, story_id: i32) {
        if self.story_viewers.contains_key(&story_id) {
            self.story_viewers_open = Some(story_id);
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.story_views(story_id).await {
                Ok(views) => {
                    let _ = tx.send(BgEvent::StoryViewsLoaded { story_id, views });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("просмотры: {e:#}")));
                }
            }
            match tg.story_viewers(story_id, 20).await {
                Ok((count, viewers)) => {
                    let names = viewers.into_iter().map(|v| v.name).collect();
                    let _ = tx.send(BgEvent::StoryViewersLoaded {
                        story_id,
                        count,
                        viewers: names,
                    });
                }
                Err(e) => {
                    tracing::debug!("story viewers failed: {e}");
                }
            }
        });
    }

    /// Медиа-табы истории: None — всё (обычная история), иначе фото/видео/ссылки.
    pub fn select_media_tab(&mut self, chat_id: i64, kind: Option<String>) {
        self.media_tab = kind.clone();
        match kind {
            None => self.load_history(chat_id),
            Some(k) => {
                let tg = Arc::clone(&self.tg);
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    match tg.search_media(chat_id, &k, 100).await {
                        Ok(items) => {
                            let _ = tx.send(BgEvent::MediaTabLoaded { chat_id, items });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Error(format!("медиапоиск: {e:#}")));
                        }
                    }
                });
            }
        }
    }

    /// Сохранить @username из настроек.
    pub fn save_username(&mut self) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let u = self.profile_username.trim().to_string();
        self.rt.spawn(async move {
            match tg.update_username(&u).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("username обновлён".into()));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("username: {e:#}")));
                }
            }
        });
    }

    /// Загрузить новое фото профиля (файловый диалог → upload).
    pub fn upload_avatar_pick(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Фото", &["jpg", "jpeg", "png", "webp"])
            .pick_file()
        else {
            return;
        };
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.upload_avatar(&path).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("фото профиля обновлено".into()));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("аватар: {e:#}")));
                }
            }
        });
    }

    /// Удалить контакт (с подтверждением вызывающего).
    pub fn delete_contact(&mut self, user_id: i64, access_hash: i64) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.delete_contact(user_id, access_hash).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log("контакт удалён".into()));
                    match tg.contacts().await {
                        Ok(c) => {
                            let _ = tx.send(BgEvent::ContactsLoaded(c));
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("удаление контакта: {e:#}")));
                }
            }
        });
    }

    /// Создать канал (megagroup) или чат (broadcast).
    pub fn do_create_channel(&mut self) {
        let title = self.create_chat_title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let about = self.create_chat_about.trim().to_string();
        let broadcast = self.create_chat_broadcast;
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.create_channel(&title, &about, broadcast).await {
                Ok(chat_id) => {
                    let _ = tx.send(BgEvent::ChannelCreated { chat_id });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("создание: {e:#}")));
                }
            }
        });
    }

    /// Запросить инвайт-ссылку чата (окно с текстом).
    pub fn load_invite_link(&mut self, chat_id: i64) {
        self.invite_link = Some("загрузка…".into());
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.invite_link(chat_id).await {
                Ok(link) => {
                    let _ = tx.send(BgEvent::InviteLinkReady(link));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("инвайт: {e:#}")));
                }
            }
        });
    }

    /// Отменить запланированное сообщение (и убрать локально).
    pub fn toggle_scheduled_cancel(&mut self, chat_id: i64, message_id: i32) {
        self.scheduled.retain(|m| m.id != message_id);
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Err(e) = tg.cancel_scheduled(chat_id, message_id).await {
                let _ = tx.send(BgEvent::Error(format!("отмена: {e:#}")));
            }
        });
    }

    /// Выполнить подтверждённое опасное действие.
    pub fn do_confirm_action(&mut self) {
        let Some(action) = self.confirm_action.take() else {
            return;
        };
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        match action {
            ConfirmAction::ClearHistory { chat_id, revoke } => {
                self.rt.spawn(async move {
                    match tg.clear_history(chat_id, revoke).await {
                        Ok(()) => {
                            let _ = tx.send(BgEvent::Log("история очищена".into()));
                            let _ = tx.send(BgEvent::ReloadHistory { chat_id });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Error(format!("очистка: {e:#}")));
                        }
                    }
                });
            }
            ConfirmAction::LeaveChannel { chat_id } => {
                self.rt.spawn(async move {
                    match tg.leave_channel(chat_id).await {
                        Ok(()) => {
                            let _ = tx.send(BgEvent::Log("вы покинули чат".into()));
                            let _ = tx.send(BgEvent::ReloadDialogs);
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Error(format!("выход: {e:#}")));
                        }
                    }
                });
            }
            ConfirmAction::DeleteStories { story_ids } => {
                self.rt.spawn(async move {
                    match tg.delete_stories(&story_ids).await {
                        Ok(()) => {
                            let _ = tx.send(BgEvent::Log("сторис удалены".into()));
                            let _ = tx.send(BgEvent::StoriesDeleted);
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Error(format!("удаление сторис: {e:#}")));
                        }
                    }
                });
            }
        }
    }

    /// Заблокировать/разблокировать пользователя (1-1 чат).
    pub fn toggle_block(&mut self, chat_id: i64, blocked: bool) {
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.block_user(chat_id, !blocked).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(
                        if !blocked { "пользователь заблокирован".into() } else { "разблокирован".into() },
                    ));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("блок: {e:#}")));
                }
            }
        });
    }

    /// Добавить контакт по телефону (экран Контакты).
    pub fn do_add_contact(&mut self) {
        let (phone, first, last) = (
            self.contact_add_phone.trim().to_string(),
            self.contact_add_first.trim().to_string(),
            self.contact_add_last.trim().to_string(),
        );
        if phone.len() < 5 || first.is_empty() {
            self.log("нужны телефон и имя".to_string());
            return;
        }
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match tg.add_contact(&phone, &first, &last).await {
                Ok(()) => {
                    let _ = tx.send(BgEvent::Log(format!("контакт {first} добавлен")));
                    match tg.contacts().await {
                        Ok(c) => {
                            let _ = tx.send(BgEvent::ContactsLoaded(c));
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("контакт: {e:#}")));
                }
            }
        });
        self.contact_add_open = false;
        self.contact_add_phone.clear();
        self.contact_add_first.clear();
        self.contact_add_last.clear();
    }

    /// Видео-превью в пузыре (160px): кэш → файл → builtin-декодер в фоне.
    pub fn ensure_video_thumb(
        &mut self,
        ctx: &egui::Context,
        msg: &crate::telegram::MessageInfo,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.video_thumbs.get(&msg.id) {
            return Some(t.clone());
        }
        if let Some(p) = self.video_thumb_files.get(&msg.id).cloned() {
            if let Ok(img) = image::open(&p) {
                let thumb = img.resize(160, 160, image::imageops::FilterType::Triangle);
                let tex = ctx.load_texture(
                    format!("videothumb::{}", msg.id),
                    to_egui(&thumb),
                    egui::TextureOptions::default(),
                );
                self.video_thumbs.insert(msg.id, tex.clone());
                return Some(tex);
            }
        }
        if self.video_thumb_requested.insert(msg.id) {
            // Тяжёлый декод — в фоне; файл нужен download_message_media по id.
            let tg = Arc::clone(&self.tg);
            let tx = self.tx.clone();
            let (chat_id, mid) = (msg.chat_id, msg.id);
            let fname = msg.file_name.clone().unwrap_or_else(|| format!("video_{mid}.mp4"));
            self.rt.spawn(async move {
                let dir = crate::telegram::stickers_cache_dir().join("video");
                std::fs::create_dir_all(&dir).ok();
                let dest = dir.join(format!("{chat_id}_{mid}_{fname}"));
                let ok = if dest.exists() {
                    true
                } else {
                    tg.download_message_media(chat_id, mid, &dir).await.is_ok() && {
                        // download кладёт файл со своим именем — ищем свежий.
                        if !dest.exists() {
                            if let Ok(mut entries) = std::fs::read_dir(&dir) {
                                let mut best: Option<(
                                    std::time::SystemTime,
                                    std::path::PathBuf,
                                )> = None;
                                for entry in entries.flatten() {
                                    let p = entry.path();
                                    if p == dest {
                                        continue;
                                    }
                                    if let Ok(meta) = entry.metadata() {
                                        if let Ok(mtime) = meta.modified() {
                                            if best.as_ref().map(|(t, _)| *t < mtime).unwrap_or(true) {
                                                best = Some((mtime, p));
                                            }
                                        }
                                    }
                                }
                                if let Some((_, src)) = best {
                                    std::fs::rename(&src, &dest).ok();
                                }
                            }
                        }
                        dest.exists()
                    }
                };
                if !ok {
                    return;
                }
                // Постер первого кадра встроенными декодерами (без ffmpeg).
                let poster = dest.with_extension("poster.jpg");
                if !poster.exists() {
                    if let Some(frame) =
                        tokio::task::spawn_blocking({
                            let dest = dest.clone();
                            move || crate::media::video::first_frame_builtin(&dest)
                        })
                        .await
                        .ok()
                        .flatten()
                    {
                        crate::media::video::save_poster_jpeg(&frame, &poster);
                    }
                }
                if poster.exists() {
                    let _ = tx.send(BgEvent::VideoThumbReady { message_id: mid, path: poster });
                }
            });
        }
        None
    }

    /// Фото веб-превью (80px): кэш → файл → скачивание в фоне.
    pub fn ensure_web_thumb(
        &mut self,
        ctx: &egui::Context,
        photo: &crate::telegram::WebPreviewPhoto,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.web_thumbs.get(&photo.photo_id) {
            return Some(t.clone());
        }
        if let Some(p) = self.web_thumb_files.get(&photo.photo_id).cloned() {
            if let Ok(img) = image::open(&p) {
                let thumb = img.resize(80, 80, image::imageops::FilterType::Triangle);
                let tex = ctx.load_texture(
                    format!("webthumb::{}", photo.photo_id),
                    to_egui(&thumb),
                    egui::TextureOptions::default(),
                );
                self.web_thumbs.insert(photo.photo_id, tex.clone());
                return Some(tex);
            }
        }
        if self.web_thumb_requested.insert(photo.photo_id) {
            let tg = Arc::clone(&self.tg);
            let tx = self.tx.clone();
            let ph = photo.clone();
            self.rt.spawn(async move {
                let dir = crate::telegram::stickers_cache_dir().join("web");
                std::fs::create_dir_all(&dir).ok();
                let dest = dir.join(format!("{}.jpg", ph.photo_id));
                if !dest.exists() {
                    if tg.download_web_photo(&ph, &dest).await.is_err() {
                        return;
                    }
                }
                if dest.exists() {
                    let _ = tx.send(BgEvent::WebThumbReady { photo_id: ph.photo_id, path: dest });
                }
            });
        }
        None
    }

    /// Одна фоновая итерация с актуальным снапшотом настроек.
    pub fn run_background_once(&mut self) {
        let engine = crate::sync::SyncEngine {
            cfg_snapshot: self.cfg.clone(),
            tg: Arc::clone(&self.tg),
            db: Arc::clone(&self.db),
            tx: self.tx.clone(),
        };
        let tx2 = self.tx.clone();
        self.rt.spawn(async move {
            let (s, d) = engine.run_once().await;
            let _ = tx2.send(BgEvent::SyncDone { stories: s, downloads: d });
        });
    }

    /// Перезапустить фоновый тикер (после смены настроек).
    pub fn restart_background_sync(&mut self) {
        if let Some(h) = self.sync_abort.take() {
            h.abort();
        }
        if !self.cfg.sync.enabled {
            return;
        }
        let secs = self.cfg.sync.interval_secs.max(15);
        let tx = self.tx.clone();
        let handle = self.rt.spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(secs));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let _ = tx.send(BgEvent::SyncTick);
            }
        });
        self.sync_abort = Some(handle.abort_handle());
    }

    /// Автозагрузка всего медиа текущего чата по правилам (для закрытых групп).
    pub fn download_all_matching(&mut self, chat_id: i64, chat_label: String) {
        let items = self.history.get(&chat_id).cloned().unwrap_or_default();
        let media: Vec<MessageInfo> = items.into_iter().filter(|m| m.has_media).collect();
        if media.is_empty() {
            self.log("нет медиа для автозагрузки");
            return;
        }
        self.log(format!(
            "автозагрузка: {} файлов из {chat_label} (параллельно, multi-DC)…",
            media.len()
        ));
        let downloader = self.make_downloader();
        let tg = Arc::clone(&self.tg);
        let tx = self.tx.clone();
        let label = chat_label.clone();
        self.rt.spawn(async move {
            let jobs: Vec<crate::media::DownloadJob> = media
                .into_iter()
                .map(|m| crate::media::DownloadJob {
                    chat_id,
                    chat_label: label.clone(),
                    message_id: m.id,
                    file_name: m.file_name.clone(),
                    media_kind: m.media_kind.clone(),
                    file_size: m.file_size,
                })
                .collect();
            for (job, res) in crate::media::download_many(&downloader, tg, jobs, 4).await {
                match res {
                    Ok(rep) => {
                        let mut s =
                            format!("сохранено: {} (msg {})", rep.local_path.display(), job.message_id);
                        if rep.duplicate {
                            s.push_str(" (дубль по хэшу)");
                        }
                        if !rep.near_duplicate_of.is_empty() {
                            s.push_str(&format!(" (≈ похож на {})", rep.near_duplicate_of));
                        }
                        if !rep.yandex_path.is_empty() {
                            s.push_str(&format!(" → Яндекс: {}", rep.yandex_path));
                        }
                        if rep.calibre_imported {
                            s.push_str(" → Calibre ✓");
                        }
                        let _ = tx.send(BgEvent::Log(s));
                    }
                    Err(e) => {
                        let _ = tx.send(BgEvent::Error(format!(
                            "скачивание msg {}: {e:#}",
                            job.message_id
                        )));
                    }
                }
            }
        });
    }

    pub fn make_downloader(&self) -> AutoDownloader {
        AutoDownloader::new(
            self.cfg.downloader.clone(),
            self.yandex_token.clone(),
            self.yandex_folder.clone(),
            self.yandex_auto,
            self.calibre_lib.clone(),
            self.calibre_auto,
            self.calibre_delete,
            Arc::clone(&self.db),
        )
        .with_progress(self.tx.clone())
        .with_workers(self.cfg.downloader.parallel_chunks)
    }

    pub fn download_message(&mut self, chat_id: i64, chat_label: String, msg: MessageInfo) {
        if !msg.has_media {
            self.log("в сообщении нет медиа");
            return;
        }
        let tx = self.tx.clone();
        let tg = Arc::clone(&self.tg);
        let downloader = self.make_downloader();
        self.log(format!("качаю msg {} из {chat_label}…", msg.id));
        self.rt.spawn(async move {
            match downloader
                .download_one(
                    tg.as_ref(),
                    chat_id,
                    &chat_label,
                    msg.id,
                    msg.file_name.clone(),
                    msg.media_kind.clone(),
                    msg.file_size,
                )
                .await
            {
                Ok(rep) => {
                    let mut s = format!("сохранено: {}", rep.local_path.display());
                    if rep.duplicate {
                        s.push_str(" (дубль по хэшу)");
                    }
                    if !rep.near_duplicate_of.is_empty() {
                        s.push_str(&format!(" (≈ похож на {})", rep.near_duplicate_of));
                    }
                    if !rep.yandex_path.is_empty() {
                        s.push_str(&format!(" → Яндекс: {}", rep.yandex_path));
                    }
                    if rep.calibre_imported {
                        s.push_str(" → Calibre ✓");
                    }
                    let _ = tx.send(BgEvent::Log(s));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("скачивание msg {}: {e:#}", msg.id)));
                }
            }
        });
    }

    /// Повторная заливка уже скачанного файла на Яндекс Диск (стриминг + прогресс).
    pub fn reupload_to_yandex(&mut self, local_path: String) {
        let token = self.yandex_token.clone();
        let folder = self.yandex_folder.clone();
        let tx = self.tx.clone();
        let tx_progress = self.tx.clone();
        let db = Arc::clone(&self.db);
        self.uploads.insert(local_path.clone(), (0, 1));
        self.rt.spawn(async move {
            let path = PathBuf::from(&local_path);
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            let remote = format!("{}/{name}", folder.trim_end_matches('/'));
            // Лимит параллельных заливок — тот же семафор, что и у автозагрузчика.
            let _permit = crate::media::upload_semaphore().acquire().await;
            let key = local_path.clone();
            let on_progress = move |done: u64, total: u64| {
                let _ = tx_progress.send(BgEvent::UploadProgress {
                    local_path: key.clone(),
                    done,
                    total,
                });
            };
            match crate::integrations::yandex::upload_file_with_progress(
                &token,
                &path,
                &remote,
                on_progress,
            )
            .await
            {
                Ok(_) => {
                    db.mark_yandex(&local_path, &remote);
                    let _ = tx.send(BgEvent::Log(format!("Яндекс: залито {remote}")));
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("Яндекс: {e:#}")));
                }
            }
        });
    }

    /// Повторный импорт книги в Calibre.
    pub fn reimport_to_calibre(&mut self, local_path: String) {
        let lib = self.calibre_lib.clone();
        let delete = self.calibre_delete;
        let tx = self.tx.clone();
        let db = Arc::clone(&self.db);
        self.rt.spawn(async move {
            let path = PathBuf::from(&local_path);
            match crate::integrations::calibre::import_book(&lib, &path, delete).await {
                Ok(out) => {
                    let _ = tx.send(BgEvent::Log(format!("Calibre: {out:?} → {local_path}")));
                    db.mark_calibre(&local_path);
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::Error(format!("Calibre: {e:#}")));
                }
            }
        });
    }

    /// Пересчёт визуальных дублей по сохранённым загрузкам (backfill):
    /// досчитывает недостающие pHash (картинки — напрямую, видео — первый кадр
    /// builtin-декодерами + сохранение постера для UI-превью) и проставляет
    /// `near_duplicate_of` через BK-дерево. Фоновая задача, прогресс в журнале.
    pub fn backfill_phashes(&mut self) {
        let db = Arc::clone(&self.db);
        let tx = self.tx.clone();
        self.log("пересчёт дублей запущен (фон)…");
        self.rt.spawn(async move {
            let rows = db.recent_downloads(5000);
            // Уже известные хэши — в индекс сразу, чтобы новые искали среди них.
            let mut tree = crate::ai::bktree::BkTree::<String>::new();
            for r in &rows {
                if let Some(h) = r.phash {
                    tree.insert(h as u64, r.local_path.clone());
                }
            }
            let mut checked = 0usize;
            let mut updated = 0usize;
            for r in &rows {
                if !std::path::Path::new(&r.local_path).exists() {
                    continue;
                }
                let need_hash = r.phash.is_none();
                let need_near = r.phash.is_some() && r.near_duplicate_of.is_empty();
                if !need_hash && !need_near {
                    continue;
                }
                // Существующий хэш переиспользуем без декода; недостающий —
                // тяжёлым путём в blocking-пуле с таймаутом.
                let ph: Option<u64> = if need_hash {
                    let lp = r.local_path.clone();
                    let work = tokio::task::spawn_blocking(move || -> Option<u64> {
                        if let Ok(img) = image::open(&lp) {
                            return Some(crate::ai::phash::phash(&img));
                        }
                        let path = std::path::Path::new(&lp);
                        let frame = crate::media::video::first_frame_builtin(path)?;
                        let poster = crate::media::video::poster_path_for(path);
                        if !crate::media::video::poster_fresh(path, &poster) {
                            crate::media::video::save_poster_jpeg(&frame, &poster);
                        }
                        Some(crate::ai::phash::phash(&frame))
                    });
                    match tokio::time::timeout(std::time::Duration::from_secs(60), work).await
                    {
                        Ok(Ok(h)) => h,
                        _ => continue,
                    }
                } else {
                    r.phash.map(|h| h as u64)
                };
                checked += 1;
                if let Some(h) = ph {
                    let near = tree
                        .query_cloned(h, crate::ai::phash::MAX_HAMMING)
                        .into_iter()
                        .find(|(other, _)| other != &r.local_path)
                        .map(|(other, _)| other)
                        .unwrap_or_default();
                    db.update_phash(&r.local_path, Some(h as i64), &near);
                    tree.insert(h, r.local_path.clone());
                    updated += 1;
                }
                if checked % 50 == 0 {
                    let _ = tx.send(BgEvent::Log(format!("пересчёт дублей: {checked}…")));
                }
            }
            let _ = tx.send(BgEvent::Log(format!(
                "пересчёт дублей готов: проверено {checked}, обновлено {updated}"
            )));
        });
    }

    pub fn sync_form_to_cfg(&mut self) {
        self.cfg.sync.enabled = self.sync_enabled;
        self.cfg.sync.interval_secs = self.sync_interval.parse().unwrap_or(120).max(15);
        self.cfg.sync.auto_download_new = self.sync_auto_new;
        self.cfg.sync.notify_on_download = self.notify_dl;
        self.cfg.sync.notify_on_ad = self.notify_ad;
    }

    pub fn profiles_to_cfg(&mut self) {
        // Синхронизируем редактируемые поля активного профиля обратно.
        if let Some(p) = self.profiles.iter_mut().find(|p| p.name == self.active_profile) {
            p.api_id = self.api_id.trim().parse().unwrap_or(p.api_id);
            p.api_hash = self.api_hash.trim().to_string();
            p.phone = self.phone.trim().to_string();
        }
        self.cfg.profiles = self.profiles.clone();
        self.cfg.active_profile = self.active_profile.clone();
        // Для совместимости держим и `telegram`.
        self.cfg.telegram = self
            .profiles
            .iter()
            .find(|p| p.name == self.active_profile)
            .cloned();
    }

    /// Перенести весь AppConfig в редактируемые поля (после импорта бандла).
    pub fn apply_cfg_to_app(&mut self) {
        self.cfg.ensure_profile();
        self.ad_threshold = self.cfg.stories.ad_threshold;
        self.blur_sigma = self.cfg.stories.blur_sigma;
        self.blur_only_regions = self.cfg.stories.blur_only_ad_regions;
        self.auto_hide_ads = self.cfg.stories.auto_hide_ads;
        self.clip_enabled = self.cfg.stories.clip_enabled;
        self.onnx_enabled = self.cfg.stories.onnx_enabled;
        self.onnx_model = self.cfg.stories.onnx_model_path.clone();
        self.onnx_input = self.cfg.stories.onnx_input_name.clone();
        self.onnx_provider = crate::ai::normalize_provider(&self.cfg.stories.onnx_provider).to_string();
        self.download_dir = self.cfg.downloader.download_dir.to_string_lossy().to_string();
        self.dl_cluster_threshold = self.cfg.downloader.cluster_max_dist.min(10);
        self.yandex_token = self.cfg.yandex.oauth_token.clone();
        self.yandex_folder = self.cfg.yandex.remote_folder.clone();
        self.yandex_auto = self.cfg.yandex.auto_upload;
        self.calibre_lib = self.cfg.calibre.library_path.clone();
        self.calibre_auto = self.cfg.calibre.auto_import_books;
        self.calibre_delete = self.cfg.calibre.delete_after_import;
        self.sync_enabled = self.cfg.sync.enabled;
        self.sync_interval = self.cfg.sync.interval_secs.to_string();
        self.sync_auto_new = self.cfg.sync.auto_download_new;
        self.notify_dl = self.cfg.sync.notify_on_download;
        self.notify_ad = self.cfg.sync.notify_on_ad;
        self.profiles = self.cfg.profiles.clone();
        self.active_profile = self.cfg.active_profile.clone();
        if let Some(p) = self.profiles.iter().find(|p| p.name == self.active_profile) {
            self.api_id = if p.api_id == 0 { String::new() } else { p.api_id.to_string() };
            self.api_hash = p.api_hash.clone();
            self.phone = p.phone.clone();
        }
        self.preview_requested.clear();
        self.stories.clear();
        self.history.clear();
        self.refresh_dialogs();
        self.refresh_stories();
        self.restart_background_sync();
    }

    /// Экспорт всех настроек одним файлом (диалог сохранения).
    pub fn export_settings_bundle(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("telecomfort-settings.toml")
            .add_filter("TOML", &["toml"])
            .save_file()
        {
            self.sync_form_to_cfg();
            self.profiles_to_cfg();
            // Синхронизируем ИИ-поля перед экспортом.
            self.cfg.stories.clip_enabled = self.clip_enabled;
            self.cfg.stories.onnx_enabled = self.onnx_enabled;
            self.cfg.stories.onnx_model_path = self.onnx_model.clone();
            self.cfg.stories.onnx_input_name = self.onnx_input.clone();
            self.cfg.stories.onnx_provider = crate::ai::normalize_provider(&self.onnx_provider).to_string();
            match self.cfg.export_bundle(&path) {
                Ok(_) => self.log(format!("настройки экспортированы: {}", path.display())),
                Err(e) => self.status = format!("экспорт не удался: {e:#}"),
            }
        }
    }

    /// Импорт настроек из файла (диалог открытия + применение).
    pub fn import_settings_bundle(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("TOML", &["toml"])
            .pick_file()
        {
            match AppConfig::import_bundle(&path) {
                Ok(cfg) => {
                    self.cfg = cfg;
                    self.apply_cfg_to_app();
                    self.log(format!("настройки импортированы из {}", path.display()));
                }
                Err(e) => self.status = format!("импорт не удался: {e:#}"),
            }
        }
    }

    pub fn save_settings(&mut self) {
        self.cfg.stories.ad_threshold = self.ad_threshold;
        self.cfg.stories.blur_sigma = self.blur_sigma;
        self.cfg.stories.blur_only_ad_regions = self.blur_only_regions;
        self.cfg.stories.auto_hide_ads = self.auto_hide_ads;
        self.cfg.stories.clip_enabled = self.clip_enabled;
        self.cfg.stories.onnx_enabled = self.onnx_enabled;
        self.cfg.stories.onnx_model_path = self.onnx_model.clone();
        self.cfg.stories.onnx_input_name = self.onnx_input.clone();
        self.cfg.stories.onnx_provider = crate::ai::normalize_provider(&self.onnx_provider).to_string();
        self.cfg.downloader.download_dir = PathBuf::from(&self.download_dir);
        self.cfg.downloader.cluster_max_dist = self.dl_cluster_threshold.min(10);
        self.cfg.yandex.oauth_token = self.yandex_token.clone();
        self.cfg.yandex.remote_folder = self.yandex_folder.clone();
        self.cfg.yandex.auto_upload = self.yandex_auto;
        self.cfg.calibre.library_path = self.calibre_lib.clone();
        self.cfg.calibre.auto_import_books = self.calibre_auto;
        self.cfg.calibre.delete_after_import = self.calibre_delete;
        self.sync_form_to_cfg();
        self.profiles_to_cfg();
        match self.cfg.save() {
            Ok(_) => {
                self.log("настройки сохранены");
                self.restart_background_sync();
            }
            Err(e) => self.log(format!("не удалось сохранить настройки: {e:#}")),
        }
    }

    /// Текстуры сторис: реальный кадр из кэша или демо-градиент + блюр рекламных зон.
    pub fn ensure_story_textures(&mut self, ctx: &egui::Context, idx: usize) {
        if idx >= self.stories.len() {
            return;
        }
        let card = &mut self.stories[idx];
        if card.texture.is_some() {
            return;
        }
        let dynimg = load_story_image(card);
        let color = to_egui(&dynimg);
        let tex = ctx.load_texture(
            format!("story{}_{}_orig", card.info.peer_id, card.info.story_id),
            color,
            egui::TextureOptions::default(),
        );
        let blurred_dyn = if self.blur_only_regions && !card.score.regions.is_empty() {
            crate::ai::blur::blur_regions(&dynimg, &card.score.regions, self.blur_sigma)
        } else {
            crate::ai::blur::blur_full(&dynimg, self.blur_sigma)
        };
        let tex_b = ctx.load_texture(
            format!("story{}_{}_blur", card.info.peer_id, card.info.story_id),
            to_egui(&blurred_dyn),
            egui::TextureOptions::default(),
        );
        card.texture = Some(tex);
        card.texture_blurred = Some(tex_b);
    }

    /// Миниатюра файла для групп похожих (кэш в `dl_thumbs`, 120px по длинной
    /// стороне). Видео — через сохранённый при скачивании постер
    /// (`<имя>.poster.jpg`), иначе пометка «🎬 видео» от вызывающего.
    /// `None` — не картинка и постера нет (в группы такие не попадают).
    /// Загружается лениво: вызывается только для раскрытых групп.
    pub fn ensure_dl_thumb(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.dl_thumbs.get(path) {
            return Some(t.clone());
        }
        // Кэп миниатюр: дешёвые 120px-текстуры, но тысячи их ни к чему —
        // при переполнении чистим всё (пересоздадутся лениво по мере раскрытия групп).
        const MAX_THUMBS: usize = 512;
        if self.dl_thumbs.len() >= MAX_THUMBS {
            self.dl_thumbs.clear();
        }
        let img = image::open(path).ok().or_else(|| {
            // Видео: постер первого кадра, сохранённый при скачивании/пересчёте.
            let poster = crate::media::video::poster_path_for(std::path::Path::new(path));
            if poster.as_os_str() != path {
                image::open(&poster).ok()
            } else {
                None
            }
        })?;
        let thumb = img.resize(120, 120, image::imageops::FilterType::Triangle);
        let tex = ctx.load_texture(
            format!("dlthumb::{path}"),
            to_egui(&thumb),
            egui::TextureOptions::default(),
        );
        self.dl_thumbs.insert(path.to_string(), tex.clone());
        Some(tex)
    }
}

/// Картинка для скоринга: для фото — сам файл, для видео — постер
/// первого кадра (MTProto-тамб/встроенные H.264/HEVC/AV1/ffmpeg) или встроенная
/// обложка (covr), иначе `None` (скорим только текст+CLIP).
fn display_image_for(path: &std::path::Path) -> Option<image::DynamicImage> {
    if crate::media::video::is_video_path(path) {
        // 1. Постер (MTProto-тамб или ffmpeg-кадр).
        let poster = crate::media::video::poster_path_for(path);
        if poster.exists() {
            if let Ok(img) = image::open(&poster) {
                return Some(img.resize(270, 480, image::imageops::FilterType::Triangle));
            }
        }
        // 2. Встроенная обложка mp4 (чистый Rust, без ffmpeg).
        if let Some(img) = crate::media::video::extract_embedded_cover(path) {
            return Some(img.resize(270, 480, image::imageops::FilterType::Triangle));
        }
        return None;
    }
    image::open(path)
        .ok()
        .map(|img| img.resize(270, 480, image::imageops::FilterType::Triangle))
}

/// Сохранить скор карточки в SQLite (ad_decisions + stories_cache).
fn persist_card_score(
    db: &crate::db::Db,
    peer_id: i64,
    story_id: i32,
    caption: &str,
    preview_path: &std::path::Path,
    score: &crate::ai::AdScore,
    threshold: f32,
) {
    let key = format!("{peer_id}:{story_id}");
    db.save_ad_decision(
        &key,
        score.score,
        &score.reasons.join("; "),
        score.is_ad(threshold),
    );
    db.save_story_preview(
        peer_id,
        story_id,
        caption,
        &preview_path.to_string_lossy(),
        score.score,
        score.is_ad(threshold),
    );
}

/// Загрузить кадр сторис: фото — файл, видео — постер первого кадра
/// (встроенные H.264/HEVC/AV1/ffmpeg) или плейсхолдер с кинополосами, иначе градиент.
fn load_story_image(card: &StoryCard) -> image::DynamicImage {
    if let Some(ref p) = card.local_path {
        if crate::media::video::is_video_path(p) {
            // Реальный кадр: постер (MTProto-тамб/ffmpeg) или встроенная обложка.
            let poster = crate::media::video::poster_path_for(p);
            if poster.exists() {
                if let Ok(img) = image::open(&poster) {
                    return img.resize(270, 480, image::imageops::FilterType::Triangle);
                }
            }
            if let Some(img) = crate::media::video::extract_embedded_cover(p) {
                return img.resize(270, 480, image::imageops::FilterType::Triangle);
            }
            return video_placeholder(card.score.score);
        }
        if let Ok(img) = image::open(p) {
            // Нормализуем к портретному превью, чтобы UI не прыгал.
            let resized = img.resize(270, 480, image::imageops::FilterType::Triangle);
            return resized;
        }
    }
    gradient_placeholder(card.score.score)
}

fn gradient_placeholder(ad_score: f32) -> image::DynamicImage {
    let is_ad = ad_score >= 0.5;
    let (w, h) = (270u32, 480u32);
    let mut img = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = y as f32 / h as f32;
            let (r, g, b) = if is_ad {
                (
                    (200.0 + 55.0 * t) as u8,
                    (120.0 - 40.0 * t) as u8,
                    (60.0 + 20.0 * t) as u8,
                )
            } else {
                (
                    (70.0 + 40.0 * t) as u8,
                    (140.0 + 30.0 * (x as f32 / w as f32)) as u8,
                    (180.0 - 30.0 * t) as u8,
                )
            };
            img.put_pixel(x, y, image::Rgb([r, g, b]));
        }
    }
    image::DynamicImage::ImageRgb8(img)
}

fn video_placeholder(ad_score: f32) -> image::DynamicImage {
    let mut img = gradient_placeholder(ad_score).to_rgb8();
    // Тёмная «кинополоса» сверху/снизу как маркер видео.
    let (w, h) = img.dimensions();
    for y in 0..28 {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgb([15, 15, 15]));
            img.put_pixel(x, h - 1 - y, image::Rgb([15, 15, 15]));
        }
    }
    image::DynamicImage::ImageRgb8(img)
}

fn to_egui(img: &image::DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let pixels = rgba
        .pixels()
        .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    egui::ColorImage::new([w, h], pixels)
}

/// Время сообщения: сегодня — ЧЧ:ММ, старше — ДД.ММ. Пусто при битом timestamp.
pub fn format_msg_time(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let dt = chrono::DateTime::from_timestamp(ts, 0)
        .map(|u| u.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    let today = chrono::Local::now().date_naive();
    if dt.date_naive() == today {
        dt.format("%H:%M").to_string()
    } else {
        dt.format("%d.%m").to_string()
    }
}

/// Идентикон чата 64x64 (fallback, когда фото профиля нет/не скачалось):
/// детерминированный цвет из хэша id+title + зеркальный 8x8-паттерн.
pub fn identicon(title: &str, id: i64) -> egui::ColorImage {
    const S: usize = 64;
    // FNV-1a по title+id.
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in title.bytes().chain(id.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    let hue = (h % 360) as f32;
    // HSV(насыщ. 0.5, ярк. 0.72) → RGB.
    let (br, bg, bb) = hsv_to_rgb(hue, 0.5, 0.72);
    let mut pixels = Vec::with_capacity(S * S);
    for y in 0..S {
        for x in 0..S {
            // Зеркальная колонка 0..=3 → 8x8-клетка 8px.
            let col = (x / 8).min(7 - x / 8);
            let row = y / 8;
            let bit = (h.rotate_left((row * 4 + col) as u32) >> 63) & 1;
            // Лёгкий вертикальный градиент для объёма.
            let shade = 0.9 + 0.2 * (y as f32 / S as f32);
            let (mut r, mut g, mut b) = (
                (br * shade).clamp(0.0, 255.0) as u8,
                (bg * shade).clamp(0.0, 255.0) as u8,
                (bb * shade).clamp(0.0, 255.0) as u8,
            );
            if bit == 1 {
                // Светлая клетка.
                r = r.saturating_add(90);
                g = g.saturating_add(90);
                b = b.saturating_add(90);
            }
            pixels.push(egui::Color32::from_rgb(r, g, b));
        }
    }
    egui::ColorImage::new([S, S], pixels)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    ((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
}

impl eframe::App for TelecomfortApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_events();
        // Завершённые передачи чистим глобально (а не только на экране Загрузок),
        // иначе «активность» будет вечно держать быстрый тикер.
        self.uploads.retain(|_, (d, t)| *d < (*t).max(1));
        // Адаптивный тикер для ноутбука: быстрый repaint только когда есть
        // живая анимация (прогресс передач, догружающиеся превью сторис),
        // в покое — редкий (события фона всё равно разбудят через SyncTick).
        let busy = !self.uploads.is_empty()
            || self.stories.iter().any(|c| c.preview_loading);
        let every = if busy {
            std::time::Duration::from_millis(400)
        } else {
            std::time::Duration::from_secs(2)
        };
        ui.ctx().request_repaint_after(every);

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("📨 Telecomfort");
                ui.separator();
                for (label, scr) in [
                    ("💬 Чаты", Screen::Chats),
                    ("✨ Сторис", Screen::Stories),
                    ("⬇ Загрузки", Screen::Downloads),
                    ("👥 Контакты", Screen::Contacts),
                    ("⚙ Настройки", Screen::Settings),
                ] {
                    let active = self.screen == scr;
                    if ui.selectable_label(active, label).clicked() {
                        self.screen = scr;
                        if scr == Screen::Contacts {
                            self.load_contacts();
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dot = if self.tg.is_connected() { "● live" } else { "● демо (mock)" };
                    ui.label(egui::RichText::new(format!("{dot} · {0}", self.me_label)).small().weak());
                });
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.status).small().weak());
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("синк: {}", self.last_sync))
                        .small()
                        .weak(),
                );
            });
        });

        match self.screen {
            Screen::Chats => super::views::chats::show(self, ui),
            Screen::Stories => super::views::stories::show(self, ui),
            Screen::Downloads => super::views::downloads::show(self, ui),
            Screen::Contacts => super::views::contacts::show(self, ui),
            Screen::Settings => super::views::settings::show(self, ui),
        }
        // Глобальное окно подтверждения опасных действий (с любого экрана).
        if let Some(action) = self.confirm_action.clone() {
            let (title, text): (&str, String) = match &action {
                ConfirmAction::ClearHistory { revoke, .. } => (
                    "🧹 Очистить историю?",
                    if *revoke {
                        "Удалить у всех участников (где позволяют права).".to_string()
                    } else {
                        "Очистить только у себя.".to_string()
                    },
                ),
                ConfirmAction::LeaveChannel { .. } => (
                    "🚪 Покинуть чат?",
                    "Вы больше не будете получать сообщения этого канала/супергруппы."
                        .to_string(),
                ),
                ConfirmAction::DeleteStories { story_ids } => (
                    "🗑 Удалить сторис?",
                    format!("Удалить {} шт. безвозвратно?", story_ids.len()),
                ),
            };
            egui::Window::new(title).collapsible(false).show(ui.ctx(), |ui| {
                ui.label(text);
                ui.horizontal(|ui| {
                    if ui.button("Подтвердить").clicked() {
                        self.do_confirm_action();
                    }
                    if ui.button("Отмена").clicked() {
                        self.confirm_action = None;
                    }
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identicon_is_deterministic_and_sized() {
        let a = identicon("Друзья", 2001);
        let b = identicon("Друзья", 2001);
        assert_eq!(a.size, [64, 64]);
        assert_eq!(a.pixels, b.pixels);
        // Не сплошной цвет: есть и фон, и клетки паттерна.
        let first = a.pixels[0];
        assert!(a.pixels.iter().any(|&p| p != first));
        // Разные чаты — разные иконки.
        let c = identicon("Учёба", 2002);
        assert_ne!(a.pixels, c.pixels);
        // Пустой title не паникует.
        let e = identicon("", 0);
        assert_eq!(e.size, [64, 64]);
    }

    #[test]
    fn msg_time_formats() {
        assert_eq!(format_msg_time(0), "");
        assert_eq!(format_msg_time(-5), "");
        // 2024-06-15 12:00 UTC — формат зависит от локальной зоны,
        // проверяем только структуру.
        let s = format_msg_time(1_718_443_200);
        assert!(s.contains(':') || s.contains('.'), "{s}");
        assert_eq!(s.len(), 5, "{s}");
    }
}
