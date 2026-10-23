//! Загрузка/сохранение настроек Telecomfort.
//!
//! Хранится в `~/.config/telecomfort/config.toml` (Linux/macOS)
//! или `%APPDATA%/telecomfort/config.toml` (Windows).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("Telecomfort")
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSettings {
    /// Человекочитаемое имя профиля (для нескольких аккаунтов)
    #[serde(default = "default_profile_name")]
    pub name: String,
    /// API ID с https://my.telegram.org
    #[serde(default)]
    pub api_id: i32,
    /// API HASH с https://my.telegram.org
    #[serde(default)]
    pub api_hash: String,
    /// Телефон в формате +7...
    #[serde(default)]
    pub phone: String,
    /// Путь к файлу сессии grammers
    #[serde(default = "default_session_path")]
    pub session_path: PathBuf,
}

fn default_profile_name() -> String {
    "main".into()
}

fn default_session_path() -> PathBuf {
    default_config_dir().join("session.session")
}

pub fn session_path_for(profile: &str) -> PathBuf {
    let safe: String = profile
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    default_config_dir().join(format!("session_{safe}.session"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoriesSettings {
    /// Порог 0..1, выше которого считаем рекламой
    #[serde(default = "default_ad_threshold")]
    pub ad_threshold: f32,
    /// Заблюривать только рекламные зоны (низ/верх), а не всё фото
    #[serde(default = "default_true")]
    pub blur_only_ad_regions: bool,
    /// Сила блюра
    #[serde(default = "default_blur_sigma")]
    pub blur_sigma: f32,
    /// Скрывать сторис с высокой вероятностью рекламы автоматически
    #[serde(default = "default_true")]
    pub auto_hide_ads: bool,
    /// CLIP-лайт (чистый Rust, второе мнение поверх эвристик+визуала)
    #[serde(default = "default_true")]
    pub clip_enabled: bool,
    /// ONNX-модель (требует сборки с `--features ort`)
    #[serde(default)]
    pub onnx_enabled: bool,
    /// Путь к ad_model.onnx
    #[serde(default)]
    pub onnx_model_path: String,
    /// Имя входа модели (пусто = автоопределение первого тензорного входа)
    #[serde(default)]
    pub onnx_input_name: String,
    /// Execution provider ONNX Runtime: auto/cpu/coreml/cuda/tensorrt/
    /// openvino/nnapi/directml/xnnpack. Недоступный EP молча откатывается
    /// на CPU (см. ai/onnx.rs). Старые конфиги без поля = "auto".
    #[serde(default = "default_onnx_provider")]
    pub onnx_provider: String,
}

fn default_onnx_provider() -> String {
    "auto".into()
}

fn default_ad_threshold() -> f32 {
    0.55
}
fn default_blur_sigma() -> f32 {
    8.0
}

impl Default for StoriesSettings {
    fn default() -> Self {
        Self {
            ad_threshold: default_ad_threshold(),
            blur_only_ad_regions: true,
            blur_sigma: default_blur_sigma(),
            auto_hide_ads: true,
            clip_enabled: true,
            onnx_enabled: false,
            onnx_model_path: String::new(),
            onnx_input_name: String::new(),
            onnx_provider: default_onnx_provider(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    Video,
    Audio,
    Document,
    #[default]
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutodownloadRule {
    /// ID чата (Telegram peer id) или username без @. "*" = все чаты.
    pub chat: String,
    #[serde(default)]
    pub kinds: Vec<MediaKind>,
    /// Максимальный размер файла в МБ (0 = без лимита)
    #[serde(default = "default_max_mb")]
    pub max_size_mb: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Игнорировать флаг `noforwards` (запрет копирования) и качать напрямую через MTProto.
    /// Работает только для контента, к которому у аккаунта есть доступ.
    #[serde(default = "default_true")]
    pub bypass_protected: bool,
}

fn default_max_mb() -> u64 {
    1500
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloaderSettings {
    #[serde(default = "default_download_dir")]
    pub download_dir: PathBuf,
    #[serde(default = "default_true")]
    pub auto_download_protected: bool,
    #[serde(default)]
    pub rules: Vec<AutodownloadRule>,
    /// Параллельных чанков MTProto на файл (1..=8). Применяется при следующем входе.
    #[serde(default = "default_parallel_chunks")]
    pub parallel_chunks: usize,
    /// Порог Хэмминга для группировки похожих в Загрузках (0..=10).
    /// На пометки `near_duplicate_of` при скачивании не влияет (там фиксированный MAX_HAMMING=6).
    #[serde(default = "default_cluster_max_dist")]
    pub cluster_max_dist: u32,
}

fn default_parallel_chunks() -> usize {
    4
}

fn default_cluster_max_dist() -> u32 {
    6
}

impl Default for DownloaderSettings {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            auto_download_protected: true,
            rules: vec![AutodownloadRule {
                chat: "*".into(),
                kinds: vec![MediaKind::Any],
                max_size_mb: 1500,
                enabled: true,
                bypass_protected: true,
            }],
            parallel_chunks: default_parallel_chunks(),
            cluster_max_dist: default_cluster_max_dist(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YandexSettings {
    /// OAuth-токен Яндекс ID (https://yandex.ru/dev/disk/poligon/)
    #[serde(default)]
    pub oauth_token: String,
    #[serde(default = "default_yd_folder")]
    pub remote_folder: String,
    #[serde(default)]
    pub auto_upload: bool,
}

fn default_yd_folder() -> String {
    "/Telecomfort".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CalibreSettings {
    /// Путь к библиотеке Calibre (где лежит metadata.db)
    #[serde(default)]
    pub library_path: String,
    #[serde(default)]
    pub auto_import_books: bool,
    /// Удалять исходный файл после импорта (по умолчанию оставляем)
    #[serde(default)]
    pub delete_after_import: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettings {
    /// Включить фоновую синхронизацию (диалоги/сторис/автозагрузка)
    #[serde(default)]
    pub enabled: bool,
    /// Интервал фонового опроса в секундах
    #[serde(default = "default_sync_interval")]
    pub interval_secs: u64,
    /// Автоматически качать новые медиа по правилам при фоновом синке
    #[serde(default = "default_true")]
    pub auto_download_new: bool,
    /// Показывать десктоп-уведомления о новой рекламе/загрузках
    #[serde(default = "default_true")]
    pub notify_on_download: bool,
    #[serde(default)]
    pub notify_on_ad: bool,
}

fn default_sync_interval() -> u64 {
    120
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 120,
            auto_download_new: true,
            notify_on_download: true,
            notify_on_ad: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Активный профиль (устаревшее поле `telegram` мигрирует сюда).
    #[serde(default)]
    pub telegram: Option<TelegramSettings>,
    /// Несколько аккаунтов. Первый = активный по умолчанию.
    #[serde(default)]
    pub profiles: Vec<TelegramSettings>,
    #[serde(default)]
    pub active_profile: String,
    #[serde(default)]
    pub stories: StoriesSettings,
    #[serde(default)]
    pub downloader: DownloaderSettings,
    #[serde(default)]
    pub yandex: YandexSettings,
    #[serde(default)]
    pub calibre: CalibreSettings,
    #[serde(default)]
    pub sync: SyncSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            telegram: None,
            profiles: vec![],
            active_profile: "main".into(),
            stories: StoriesSettings::default(),
            downloader: DownloaderSettings::default(),
            yandex: YandexSettings::default(),
            calibre: CalibreSettings::default(),
            sync: SyncSettings::default(),
        }
    }
}

pub fn default_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("telecomfort")
}

pub fn config_file_path() -> PathBuf {
    default_config_dir().join("config.toml")
}

impl AppConfig {
    /// Активный Telegram-профиль (с миграцией старого `telegram`).
    pub fn active_telegram(&self) -> Option<&TelegramSettings> {
        if let Some(p) = self.profiles.iter().find(|p| p.name == self.active_profile) {
            return Some(p);
        }
        if let Some(first) = self.profiles.first() {
            return Some(first);
        }
        self.telegram.as_ref()
    }

    pub fn active_telegram_mut(&mut self) -> Option<&mut TelegramSettings> {
        if self.profiles.iter().any(|p| p.name == self.active_profile) {
            let name = self.active_profile.clone();
            return self.profiles.iter_mut().find(|p| p.name == name);
        }
        if !self.profiles.is_empty() {
            return self.profiles.first_mut();
        }
        self.telegram.as_mut()
    }

    /// Гарантировать наличие хотя бы одного профиля (для UI).
    pub fn ensure_profile(&mut self) -> &mut TelegramSettings {
        if self.profiles.is_empty() {
            if let Some(t) = self.telegram.take() {
                let mut t = t;
                if t.name.is_empty() {
                    t.name = "main".into();
                }
                if t.session_path.as_os_str().is_empty() {
                    t.session_path = session_path_for(&t.name);
                }
                self.active_profile = t.name.clone();
                self.profiles.push(t);
            } else {
                self.profiles.push(TelegramSettings {
                    name: "main".into(),
                    api_id: 0,
                    api_hash: String::new(),
                    phone: String::new(),
                    session_path: session_path_for("main"),
                });
                self.active_profile = "main".into();
            }
        }
        let name = self.active_profile.clone();
        let idx = self
            .profiles
            .iter()
            .position(|p| p.name == name)
            .unwrap_or(0);
        &mut self.profiles[idx]
    }

    pub fn load() -> Result<Self> {
        let path = config_file_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let mut cfg: Self = toml::from_str(&text).context("parse config.toml")?;
        // Миграция: старый одиночный `telegram` → `profiles`.
        if cfg.profiles.is_empty() {
            if let Some(mut t) = cfg.telegram.clone() {
                if t.name.is_empty() {
                    t.name = "main".into();
                }
                // Старый путь session.session → session_main.session для консистентности.
                cfg.profiles.push(t);
                if cfg.active_profile.is_empty() {
                    cfg.active_profile = "main".into();
                }
            }
        }
        if cfg.sync.interval_secs < 15 {
            cfg.sync.interval_secs = 15;
        }
        Ok(cfg)
    }

    #[cfg(test)]
    pub fn sample_for_tests() -> Self {
        let mut cfg = Self::default();
        cfg.ensure_profile();
        cfg
    }

    pub fn save(&self) -> Result<()> {
        let dir = default_config_dir();
        std::fs::create_dir_all(&dir).context("create config dir")?;
        let path = config_file_path();
        Self::save_to(self, &path)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let text = toml::to_string_pretty(self).context("serialize config")?;
        std::fs::write(path, text).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    /// Экспорт всех настроек одним файлом (TOML). Включает профили,
    /// правила, сторис/синк/яндекс/calibre. Токены — внутри файла,
    /// храните его локально.
    pub fn export_bundle(&self, path: &std::path::Path) -> Result<()> {
        Self::save_to(self, path)
    }

    /// Импорт бандла настроек: парсинг + мягкая валидация + сохранение
    /// в стандартное место. Возвращает загруженный конфиг.
    pub fn import_bundle(path: &std::path::Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let mut cfg: Self = toml::from_str(&text).context(
            "не похож на файл настроек Telecomfort (ожидается TOML из «Экспорт настроек»)",
        )?;
        if cfg.sync.interval_secs < 15 {
            cfg.sync.interval_secs = 15;
        }
        if cfg.active_profile.is_empty() {
            cfg.active_profile = "main".into();
        }
        cfg.ensure_profile();
        cfg.save()?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_export_import_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bundle.toml");
        let mut cfg = AppConfig::sample_for_tests();
        cfg.stories.ad_threshold = 0.77;
        cfg.stories.clip_enabled = true;
        cfg.stories.onnx_model_path = "/tmp/ad.onnx".into();
        cfg.stories.onnx_provider = "coreml".into();
        cfg.downloader.rules.push(AutodownloadRule {
            chat: "test-chat".into(),
            kinds: vec![MediaKind::Video],
            max_size_mb: 100,
            enabled: true,
            bypass_protected: true,
        });
        cfg.export_bundle(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let parsed: AppConfig = toml::from_str(&text).unwrap();
        assert!((parsed.stories.ad_threshold - 0.77).abs() < 1e-6);
        assert!(parsed.stories.clip_enabled);
        assert_eq!(parsed.stories.onnx_provider, "coreml");
        assert!(parsed.downloader.rules.iter().any(|r| r.chat == "test-chat"));
    }

    #[test]
    fn import_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.toml");
        std::fs::write(&file, "this is [ not toml").unwrap();
        assert!(AppConfig::import_bundle(&file).is_err());
    }
}
