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
    Settings,
}

pub struct StoryCard {
    pub info: StoryInfo,
    pub score: crate::ai::AdScore,
    pub revealed: bool,
    pub texture: Option<egui::TextureHandle>,
    pub texture_blurred: Option<egui::TextureHandle>,
    /// Локальный кадр (реальный PNG/JPG из кэша или сгенерированный mock).
    pub local_path: Option<PathBuf>,
    pub preview_loading: bool,
    pub visual_done: bool,
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
    pub uploads: HashMap<String, (u64, u64)>,
    /// Кэш миниатюр для групп похожих (путь → текстура 120px).
    /// Чистится implicitly: ключ — путь файла, stale-превью приемлемы.
    pub dl_thumbs: HashMap<String, egui::TextureHandle>,

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
            dl_thumbs: HashMap::new(),
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
    /// стороне). `None` — файл не открывается как картинка (видео/документы
    /// в группы не попадают, т.к. у них нет pHash — см. `AutoDownloader`).
    /// Загружается лениво: вызывается только для раскрытых групп.
    pub fn ensure_dl_thumb(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.dl_thumbs.get(path) {
            return Some(t.clone());
        }
        // Не плодим текстуры без нужды: кэш ограничен сотнями записей
        // (группы обычно — единицы файлов; старые записи не чистим —
        // TextureHandle дешёвый, GPU-память под 120px-миниатюры мизерная).
        let img = image::open(path).ok()?;
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

impl eframe::App for TelecomfortApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_events();
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("📨 Telecomfort");
                ui.separator();
                for (label, scr) in [
                    ("💬 Чаты", Screen::Chats),
                    ("✨ Сторис", Screen::Stories),
                    ("⬇ Загрузки", Screen::Downloads),
                    ("⚙ Настройки", Screen::Settings),
                ] {
                    let active = self.screen == scr;
                    if ui.selectable_label(active, label).clicked() {
                        self.screen = scr;
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
            Screen::Settings => super::views::settings::show(self, ui),
        }
    }
}
