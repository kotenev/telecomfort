//! Telegram-слой: типы, трейт сервиса, mock для офлайн-демо и grammers-бэкенд.
//!
//! Почему grammers (MTProto user-клиент), а не Bot API:
//! - Сторис доступны только user-клиентам;
//! - Скачивание из групп с `noforwards` (запрет копирования/пересылки) ботом
//!   невозможно, а user-клиентом — скачивается напрямую через
//!   `upload.getFile` по file reference. Флаг `noforwards` — это лишь
//!   указание официальным клиентам скрыть кнопки «скопировать/переслать»,
//!   сервер отдаёт контент тому, у кого есть доступ к чату.

pub mod backend_grammers;
pub mod mock;
pub mod session;

pub use mock::MockTelegram;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatInfo {
    pub id: i64,
    pub title: String,
    pub username: Option<String>,
    /// Запрет копирования/пересылки (noforwards)
    pub protected: bool,
    pub unread: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: i32,
    pub chat_id: i64,
    pub text: String,
    pub date: i64,
    pub has_media: bool,
    pub media_kind: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    /// Исходящее (отправлено мной) — пузырь справа, доступны ✏️/🗑.
    pub outgoing: bool,
    /// Имя отправителя (для групп/каналов).
    pub sender: Option<String>,
    /// Было отредактировано (метка «изм.»).
    pub edited: bool,
    /// Суммарное число реакций (None — нет информации/реакций).
    pub reactions: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryInfo {
    pub peer_id: i64,
    pub peer_title: String,
    pub story_id: i32,
    pub caption: String,
    pub has_url: bool,
    pub date: i64,
    /// Для демо/кэша: путь к превью или пусто
    pub preview_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoginState {
    pub phone: String,
    pub code_hash: String,
    pub need_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: i64,
    pub name: String,
    pub phone: Option<String>,
    pub username: Option<String>,
}

/// Папка кэша превью сторис: `~/.cache/telecomfort/stories` (или TEMP на Windows).
pub fn stories_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("telecomfort")
        .join("stories")
}

/// Папка кэша аватаров диалогов: `~/.cache/telecomfort/avatars`.
/// Файлы `<bot_api_id>.jpg` (лучший доступный размер фото профиля).
/// Группы/каналы без доступного фото — UI рисует идентикон (цвет+паттерн от id).
pub fn avatars_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("telecomfort")
        .join("avatars")
}

#[async_trait]
pub trait TelegramService: Send + Sync {
    async fn dialogs(&self) -> anyhow::Result<Vec<ChatInfo>>;
    async fn history(&self, chat_id: i64, limit: usize) -> anyhow::Result<Vec<MessageInfo>>;
    async fn stories(&self) -> anyhow::Result<Vec<StoryInfo>>;
    /// Скачать медиа сообщения в `dest_dir`. Возвращает путь к файлу.
    /// Реализация обязана игнорировать UI-флаг `noforwards` и качать напрямую.
    async fn download_message_media(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf>;
    /// Скачать кадр сторис (фото/видео-превью) в `dest_dir`.
    /// Mock генерирует PNG-заглушку, live-бэкенд качает через
    /// `stories.getStoriesByID` + `download_media`.
    async fn download_story_media(
        &self,
        peer_id: i64,
        story_id: i32,
        dest_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf>;
    /// Скачать медиа сообщения с прогрессом `done/total` и докачкой.
    /// `workers` — параллельных чанков (1 = последовательно).
    /// Дефолт: plain-загрузка + финальный тик 100%.
    async fn download_message_media_with_progress(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
        workers: usize,
        on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> anyhow::Result<std::path::PathBuf> {
        let _ = (workers, on_progress);
        self.download_message_media(chat_id, message_id, dest_dir).await
    }
    /// Информация о текущем аккаунте (для мультиаккаунтов).
    async fn me(&self) -> anyhow::Result<AccountInfo> {
        anyhow::bail!("not implemented for this backend")
    }
    /// Отправить текст (возвращает id отправленного). `reply_to` — ответ на сообщение.
    /// Дефолт: не поддерживается (mock переопределяет для демо).
    async fn send_text(
        &self,
        _chat_id: i64,
        _text: &str,
        _reply_to: Option<i32>,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("send is not implemented for this backend")
    }
    /// Отредактировать своё сообщение (только текст).
    async fn edit_text(&self, _chat_id: i64, _message_id: i32, _text: &str) -> anyhow::Result<()> {
        anyhow::bail!("edit is not implemented for this backend")
    }
    /// Удалить сообщения (revoke — у всех).
    async fn delete_messages(&self, _chat_id: i64, _message_ids: &[i32]) -> anyhow::Result<()> {
        anyhow::bail!("delete is not implemented for this backend")
    }
    /// Поставить/убрать реакцию (пустой emoji — убрать).
    async fn react(&self, _chat_id: i64, _message_id: i32, _emoji: &str) -> anyhow::Result<()> {
        anyhow::bail!("reactions are not implemented for this backend")
    }
    /// Отметить диалог прочитанным. Дефолт — no-op (mock).
    async fn mark_read(&self, _chat_id: i64) -> anyhow::Result<()> {
        Ok(())
    }
    /// Отправить файл (фото — как фото, остальное — документом), caption — подпись.
    /// Возвращает id отправленного.
    async fn send_file(
        &self,
        _chat_id: i64,
        _path: &std::path::Path,
        _caption: &str,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("file sending is not implemented for this backend")
    }
    /// Путь к закэшированному аватару чата (`<id>.jpg`), если есть/скачался.
    /// Дефолт — нет фото (UI рисует идентикон).
    async fn avatar_path(&self, _chat_id: i64) -> anyhow::Result<Option<std::path::PathBuf>> {
        Ok(None)
    }
    fn is_connected(&self) -> bool;
}
