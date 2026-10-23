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
    /// Чат заглушён (mute_until в будущем).
    pub muted: bool,
    /// Чат в архиве (папка 1).
    pub archived: bool,
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
    /// Просмотры (каналы) и пересылки, если сервер отдал.
    pub views: Option<i32>,
    pub forwards: Option<i32>,
    /// Опрос (вопрос + варианты + голоса), если это poll.
    pub poll: Option<PollInfo>,
    /// Геопозиция (lat, long), если сообщение — точка на карте.
    pub geo: Option<(f64, f64)>,
    /// Контакт (имя, телефон), если сообщение — карточка контакта.
    pub contact: Option<(String, String)>,
}

/// Один вариант ответа опроса.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollAnswerInfo {
    pub text: String,
    /// Байты option для `messages.sendVote`.
    #[serde(with = "serde_bytes_vec")]
    pub option: Vec<u8>,
    pub votes: Option<i32>,
    pub chosen: bool,
}

/// Опрос целиком (для карточки + голосования).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollInfo {
    pub question: String,
    pub closed: bool,
    pub quiz: bool,
    pub total_voters: Option<i32>,
    pub answers: Vec<PollAnswerInfo>,
}

/// Контакт из телефонной книги Telegram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactInfo {
    pub name: String,
    pub phone: String,
    pub username: Option<String>,
    pub user_id: i64,
}

/// Папка диалогов сервера (вкладка-фильтр).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogFilterInfo {
    pub id: i32,
    pub title: String,
    pub peer_ids: Vec<i64>,
}

/// Активная сессия (`account.getAuthorizations`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub current: bool,
    pub app_name: String,
    pub device: String,
    pub platform: String,
    pub country: String,
    pub ip: String,
    pub last_active: i64,
    pub hash: i64,
}

/// Профиль текущего аккаунта (имя + о себе).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileInfo {
    pub first_name: String,
    pub last_name: String,
    pub about: String,
}

/// Сериализация `Vec<u8>` как массива чисел (serde без extra-фич).
mod serde_bytes_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        v.to_vec().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Vec::<u8>::deserialize(d)
    }
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

/// Папка кэша голосовых/аудио для плеера: `~/.cache/telecomfort/audio`.
/// Файлы `<chat>_<msg>.<ext>`, качаются по требованию (не попадают в Загрузки/БД).
pub fn audio_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("telecomfort")
        .join("audio")
}

/// Папка кэша стикеров для превью в пузырях: `~/.cache/telecomfort/stickers`.
/// Файлы `sticker_<msg>.webp`, качаются по тапу (не попадают в Загрузки/БД).
pub fn stickers_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("telecomfort")
        .join("stickers")
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
    /// Закрепить сообщение в чате (сервер проверяет права).
    async fn pin_message(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        anyhow::bail!("pins are not implemented for this backend")
    }
    /// Открепить сообщение в чате.
    async fn unpin_message(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        anyhow::bail!("pins are not implemented for this backend")
    }
    /// Закреплённое сообщение чата (поиск по фильтру pinned, лимит 1).
    async fn pinned_message(&self, _chat_id: i64) -> anyhow::Result<Option<MessageInfo>> {
        Ok(None)
    }
    /// Проголосовать в опросе (один вариант; для multiple-choice — по одному за раз).
    async fn vote(&self, _chat_id: i64, _message_id: i32, _option: Vec<u8>) -> anyhow::Result<()> {
        anyhow::bail!("polls are not implemented for this backend")
    }
    /// Переслать сообщения в другой чат. Возвращает число пересланных.
    async fn forward(
        &self,
        _from_chat: i64,
        _message_ids: &[i32],
        _to_chat: i64,
    ) -> anyhow::Result<usize> {
        anyhow::bail!("forward is not implemented for this backend")
    }
    /// Заглушить/разглушить чат (mute_until в будущее/0).
    async fn mute(&self, _chat_id: i64, _muted: bool) -> anyhow::Result<()> {
        anyhow::bail!("mute is not implemented for this backend")
    }
    /// В архив/из архива (папка 1/0).
    async fn archive(&self, _chat_id: i64, _archived: bool) -> anyhow::Result<()> {
        anyhow::bail!("archive is not implemented for this backend")
    }
    /// Папки диалогов сервера (вкладки-фильтры).
    async fn dialog_filters(&self) -> anyhow::Result<Vec<DialogFilterInfo>> {
        Ok(vec![])
    }
    /// Контакты телефонной книги Telegram.
    async fn contacts(&self) -> anyhow::Result<Vec<ContactInfo>> {
        Ok(vec![])
    }
    /// Отложенная отправка (сервер доставит в `at`). Возвращает id.
    async fn send_scheduled(
        &self,
        _chat_id: i64,
        _text: &str,
        _reply_to: Option<i32>,
        _at: std::time::SystemTime,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("scheduled send is not implemented for this backend")
    }
    /// Сохранить черновик на сервере (синк между устройствами).
    async fn save_draft(&self, _chat_id: i64, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }
    /// Активные сессии текущего аккаунта.
    async fn sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        Ok(vec![])
    }
    /// Завершить все сессии, кроме текущей.
    async fn terminate_others(&self) -> anyhow::Result<()> {
        anyhow::bail!("sessions are not implemented for this backend")
    }
    /// Профиль текущего аккаунта (имя + о себе).
    async fn get_profile(&self) -> anyhow::Result<ProfileInfo> {
        Ok(ProfileInfo::default())
    }
    /// Обновить имя/о себе текущего аккаунта.
    async fn update_profile(
        &self,
        _first_name: &str,
        _last_name: &str,
        _about: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("profile edit is not implemented for this backend")
    }
    /// Опубликовать сторис (фото/видео + подпись, видно контактам).
    /// Возвращает id сторис.
    async fn publish_story(
        &self,
        _path: &std::path::Path,
        _caption: &str,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("story publishing is not implemented for this backend")
    }
    fn is_connected(&self) -> bool;
}
