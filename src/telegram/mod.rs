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
    /// Веб-превью ссылки, если есть.
    pub web_preview: Option<WebPreview>,
    /// ID топика форума (None — общий чат). Заполняется чтением топика.
    pub topic_id: Option<i32>,
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
    /// Пояснение правильного ответа (викторины).
    pub solution: Option<String>,
}

/// Контакт из телефонной книги Telegram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactInfo {
    pub name: String,
    pub phone: String,
    pub username: Option<String>,
    pub user_id: i64,
    pub access_hash: i64,
}

/// Участник чата (для списка участников групп/каналов).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantInfo {
    pub id: i64,
    pub name: String,
    pub admin: bool,
    pub access_hash: i64,
}
/// Заблокированный пользователь/чат.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedInfo {
    pub id: i64,
    pub name: String,
}

/// Тонкие права администратора канала/супергруппы (как в официальном клиенте: галки в окне).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AdminRights {
    pub change_info: bool,
    pub post_messages: bool,
    pub edit_messages: bool,
    pub delete_messages: bool,
    pub ban_users: bool,
    pub invite_users: bool,
    pub pin_messages: bool,
    pub manage_call: bool,
    pub manage_topics: bool,
    pub post_stories: bool,
    pub edit_stories: bool,
    pub delete_stories: bool,
    pub manage_direct_messages: bool,
}

impl AdminRights {
    /// Полный набор (кроме анонимности и добавления админов — как раньше).
    pub fn full() -> Self {
        Self {
            change_info: true,
            post_messages: true,
            edit_messages: true,
            delete_messages: true,
            ban_users: true,
            invite_users: true,
            pin_messages: true,
            manage_call: true,
            manage_topics: true,
            post_stories: true,
            edit_stories: true,
            delete_stories: true,
            manage_direct_messages: true,
        }
    }
    /// Пустой набор = обычный участник (разжаловать).
    pub fn none() -> Self {
        Self {
            change_info: false,
            post_messages: false,
            edit_messages: false,
            delete_messages: false,
            ban_users: false,
            invite_users: false,
            pin_messages: false,
            manage_call: false,
            manage_topics: false,
            post_stories: false,
            edit_stories: false,
            delete_stories: false,
            manage_direct_messages: false,
        }
    }
    pub fn any(&self) -> bool {
        self.change_info
            || self.post_messages
            || self.edit_messages
            || self.delete_messages
            || self.ban_users
            || self.invite_users
            || self.pin_messages
            || self.manage_call
            || self.manage_topics
            || self.post_stories
            || self.edit_stories
            || self.delete_stories
            || self.manage_direct_messages
    }
}

/// Подборка сторис (хайлайтс) текущего пользователя.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryAlbumInfo {
    pub album_id: i32,
    pub title: String,
    pub story_count: i32,
}

/// Базовая аналитика канала (без графиков).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelStats {
    pub followers: i64,
    pub followers_prev: i64,
    pub views_per_post: f64,
    pub shares_per_post: f64,
    pub reactions_per_post: f64,
    pub views_per_story: f64,
    pub notifications_pct: f64,
}

/// Полное описание канала/группы (единый запрос channels.getFullChannel).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatFullInfo {
    pub about: String,
    pub participants_count: Option<i32>,
    pub admins_count: Option<i32>,
    pub online_count: Option<i32>,
    pub linked_chat_id: Option<i64>,
    pub ttl_period: Option<i32>,
    /// Включён ли антиспам (только каналы/супергруппы).
    #[serde(default)]
    pub antispam: bool,
}

/// Полное описание пользователя (users.getFullUser + флаги).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeerFullInfo {
    pub about: String,
    pub premium: bool,
    pub verified: bool,
    pub scam: bool,
    pub fake: bool,
    pub voice_forbidden: bool,
}

/// Один проголосовавший реакцией (messages.getMessageReactionsList).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionVoter {
    pub user_id: i64,
    pub name: String,
    pub date: i64,
    pub emoji: String,
}

/// Недавний стикер (документ + превью в кэше).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerInfo {
    pub id: i64,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
    pub emoji: Option<String>,
    pub preview_path: Option<std::path::PathBuf>,
}

/// Сохранённая GIF (документ mp4 + превью-кадр в кэше).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GifInfo {
    pub id: i64,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub duration_ms: i32,
    pub preview_path: Option<std::path::PathBuf>,
}

/// Вариант Stars-подарка (только чтение тарифов).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarGiftOption {
    pub stars: i64,
    pub currency: String,
    pub amount: i64,
}

/// Подарок из каталога (payments.getStarGifts), готовый к отправке.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarGiftFull {
    pub id: i64,
    pub stars: i64,
    pub title: String,
    pub sold_out: bool,
    pub require_premium: bool,
    pub limited: bool,
    pub convert_stars: i64,
}

/// Одна Stars-транзакция (payments.getStarsTransactions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarTxInfo {
    pub tx_id: String,
    pub date: i64,
    /// Подписанная сумма: + вход, − расход.
    pub stars: i64,
    pub title: Option<String>,
    pub peer_name: Option<String>,
    pub flags: Vec<String>,
}

/// Сохранённый Stars-подарок (payments.getSavedStarGifts, только чтение).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedGiftInfo {
    pub gift_id: i64,
    pub title: String,
    pub stars: i64,
    pub date: i64,
    pub from_name: Option<String>,
    pub can_upgrade: bool,
    pub message: Option<String>,
}

/// Рядом находящийся пир (contacts.getLocated, без точной дистанции — API её не отдаёт).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyPeer {
    pub id: i64,
    pub title: String,
    pub is_user: bool,
}

/// Зритель сторис (имя для списка просмотревших).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryViewer {
    pub name: String,
}

/// Папка диалогов сервера (вкладка-фильтр).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogFilterInfo {
    pub id: i32,
    pub title: String,
    pub peer_ids: Vec<i64>,
}

/// Топик форума (`messages.getForumTopics`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicInfo {
    pub id: i32,
    pub title: String,
}

/// Веб-превью ссылки (карточка под сообщением).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    pub photo: Option<WebPreviewPhoto>,
}

/// Фото веб-превью (скачивается отдельно в кэш).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebPreviewPhoto {
    pub photo_id: i64,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
    pub dc_id: i32,
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

/// Профиль текущего аккаунта (имя + о себе + username).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileInfo {
    pub first_name: String,
    pub last_name: String,
    pub about: String,
    pub username: Option<String>,
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
        self.download_message_media(chat_id, message_id, dest_dir)
            .await
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
        _silent: bool,
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
        _silent: bool,
        _schedule_at: Option<std::time::SystemTime>,
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
        _silent: bool,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("scheduled send is not implemented for this backend")
    }
    /// Сохранить черновик на сервере (синк между устройствами).
    async fn save_draft(
        &self,
        _chat_id: i64,
        _text: &str,
        _reply_to: Option<i32>,
    ) -> anyhow::Result<()> {
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
    /// Опубликовать сторис (фото/видео + подпись + опциональная музыка, видно контактам).
    /// Возвращает id сторис.
    async fn publish_story(
        &self,
        _path: &std::path::Path,
        _caption: &str,
        _music: Option<std::path::PathBuf>,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("story publishing is not implemented for this backend")
    }
    /// Удалить свои сторис по id.
    async fn delete_stories(&self, _story_ids: &[i32]) -> anyhow::Result<()> {
        anyhow::bail!("story deletion is not implemented for this backend")
    }
    /// Топики форума чата (пусто — не форум).
    async fn forum_topics(&self, _chat_id: i64) -> anyhow::Result<Vec<TopicInfo>> {
        Ok(vec![])
    }
    /// Создать топик форума.
    async fn create_topic(&self, _chat_id: i64, _title: &str) -> anyhow::Result<i32> {
        anyhow::bail!("forum topics are not implemented for this backend")
    }
    /// История топика (сырой getHistory с top_msg_id + свой парсер).
    async fn topic_history(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _limit: usize,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        Ok(vec![])
    }
    /// Отправить текст в топик (top_msg_id).
    async fn send_to_topic(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _text: &str,
        _silent: bool,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("forum topics are not implemented for this backend")
    }
    /// Глобальный поиск по всем чатам. Возвращает (chat_id, сообщение).
    async fn global_search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<(i64, MessageInfo)>> {
        Ok(vec![])
    }
    /// Запланированные сообщения чата.
    async fn scheduled_history(&self, _chat_id: i64) -> anyhow::Result<Vec<MessageInfo>> {
        Ok(vec![])
    }
    /// Отменить запланированное сообщение.
    async fn cancel_scheduled(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        anyhow::bail!("scheduled messages are not implemented for this backend")
    }
    /// Создать группу (megagroup) или канал (broadcast). Возвращает chat_id.
    async fn create_channel(
        &self,
        _title: &str,
        _about: &str,
        _broadcast: bool,
    ) -> anyhow::Result<i64> {
        anyhow::bail!("channel creation is not implemented for this backend")
    }
    /// Пригласительная ссылка чата.
    async fn invite_link(&self, _chat_id: i64) -> anyhow::Result<String> {
        anyhow::bail!("invite links are not implemented for this backend")
    }
    /// Заблокировать/разблокировать пользователя (1-1 чат).
    async fn block_user(&self, _chat_id: i64, _blocked: bool) -> anyhow::Result<()> {
        anyhow::bail!("block is not implemented for this backend")
    }
    /// Очистить историю (just_clear — у себя, revoke — у всех; сервер проверяет права).
    async fn clear_history(&self, _chat_id: i64, _revoke: bool) -> anyhow::Result<()> {
        anyhow::bail!("clear history is not implemented for this backend")
    }
    /// Покинуть канал/супергруппу.
    async fn leave_channel(&self, _chat_id: i64) -> anyhow::Result<()> {
        anyhow::bail!("leave is not implemented for this backend")
    }
    /// Добавить контакт по телефону.
    async fn add_contact(
        &self,
        _phone: &str,
        _first_name: &str,
        _last_name: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("add contact is not implemented for this backend")
    }
    /// Скачать фото веб-превью в кэш (для карточек ссылок).
    async fn download_web_photo(
        &self,
        _photo: &WebPreviewPhoto,
        _dest: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        anyhow::bail!("web preview download is not implemented for this backend")
    }
    /// Создать опрос (quiz — с правильным ответом). Возвращает id сообщения.
    async fn create_poll(
        &self,
        _chat_id: i64,
        _question: &str,
        _options: &[String],
        _quiz: bool,
        _correct: Option<usize>,
        _anonymous: bool,
        _multiple_choice: bool,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("poll creation is not implemented for this backend")
    }
    /// Отправить геопозицию (точку на карте).
    async fn send_geo(&self, _chat_id: i64, _lat: f64, _long: f64) -> anyhow::Result<i32> {
        anyhow::bail!("geo send is not implemented for this backend")
    }
    /// Ответить на сторис текстом (ответ уходит в чат владельца).
    async fn story_reply(&self, _peer_id: i64, _story_id: i32, _text: &str) -> anyhow::Result<i32> {
        anyhow::bail!("story replies are not implemented for this backend")
    }
    /// Просмотры своей сторис (count).
    async fn story_views(&self, _story_id: i32) -> anyhow::Result<i32> {
        anyhow::bail!("story views are not implemented for this backend")
    }
    /// Стелс-режим сторис (не палить просмотры): прошлое + будущее.
    async fn enable_stealth(&self) -> anyhow::Result<()> {
        anyhow::bail!("stealth mode is not implemented for this backend")
    }
    /// Реакция на чужую сторис (emoji; повтор — toggle на сервере).
    async fn story_react(&self, _peer_id: i64, _story_id: i32, _emoji: &str) -> anyhow::Result<()> {
        anyhow::bail!("story reactions are not implemented for this backend")
    }
    /// Закрепить/открепить свою сторис в профиле.
    async fn toggle_story_pin(&self, _story_id: i32, _pinned: bool) -> anyhow::Result<()> {
        anyhow::bail!("story pins are not implemented for this backend")
    }
    /// Зрители своей сторис (первые N имён).
    async fn story_viewers(
        &self,
        _story_id: i32,
        _limit: usize,
    ) -> anyhow::Result<(i32, Vec<StoryViewer>)> {
        anyhow::bail!("story views are not implemented for this backend")
    }
    /// Медиа-табы истории: только фото / только видео / только ссылки.
    async fn search_media(
        &self,
        _chat_id: i64,
        _kind: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        Ok(vec![])
    }
    /// Изменить @username текущего аккаунта.
    async fn update_username(&self, _username: &str) -> anyhow::Result<()> {
        anyhow::bail!("username edit is not implemented for this backend")
    }
    /// Проверить доступность @username (true — свободен).
    async fn check_username(&self, _username: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    /// Закрепить/открепить диалог в списке (toggleDialogPin).
    async fn toggle_dialog_pin(&self, _chat_id: i64, _pinned: bool) -> anyhow::Result<()> {
        anyhow::bail!("dialog pins are not implemented for this backend")
    }
    /// Отправить альбом файлов (2+). Возвращает id сообщений.
    async fn send_album(
        &self,
        _chat_id: i64,
        _paths: &[std::path::PathBuf],
        _caption: &str,
        _schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<Vec<i32>> {
        anyhow::bail!("albums are not implemented for this backend")
    }
    /// Архив своих сторис.
    async fn archived_stories(&self) -> anyhow::Result<Vec<StoryInfo>> {
        Ok(vec![])
    }
    /// Загрузить новое фото профиля (fallback=false).
    async fn upload_avatar(&self, _path: &std::path::Path) -> anyhow::Result<()> {
        anyhow::bail!("avatar upload is not implemented for this backend")
    }
    /// Удалить контакт по (user_id, access_hash).
    async fn delete_contact(&self, _user_id: i64, _access_hash: i64) -> anyhow::Result<()> {
        anyhow::bail!("delete contact is not implemented for this backend")
    }
    /// Чёрный список (пользователи и чаты).
    async fn blocked_list(&self) -> anyhow::Result<Vec<BlockedInfo>> {
        Ok(vec![])
    }
    /// Поделиться контактом (карточка в чат).
    async fn share_contact(
        &self,
        _chat_id: i64,
        _phone: &str,
        _first_name: &str,
        _last_name: &str,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("contact share is not implemented for this backend")
    }
    /// Остановить свой опрос (закрыть голосование).
    async fn stop_poll(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        anyhow::bail!("stop poll is not implemented for this backend")
    }
    /// Все закреплённые сообщения чата (до limit).
    async fn pinned_list(&self, _chat_id: i64, _limit: usize) -> anyhow::Result<Vec<MessageInfo>> {
        Ok(vec![])
    }
    /// Отозвать инвайт-ссылку.
    async fn revoke_invite(&self, _chat_id: i64, _link: &str) -> anyhow::Result<()> {
        anyhow::bail!("invite revoke is not implemented for this backend")
    }
    /// Участники канала/супергруппы (первые N).
    async fn participants(
        &self,
        _chat_id: i64,
        _limit: usize,
    ) -> anyhow::Result<Vec<ParticipantInfo>> {
        Ok(vec![])
    }
    /// Исключить/забанить участника (нужны права админа).
    async fn kick_participant(&self, _chat_id: i64, _user_id: i64) -> anyhow::Result<()> {
        anyhow::bail!("kick is not implemented for this backend")
    }
    /// Скрыть/показать сторис пользователя (лента).
    async fn hide_peer_stories(&self, _peer_id: i64, _hidden: bool) -> anyhow::Result<()> {
        anyhow::bail!("hide stories is not implemented for this backend")
    }
    /// Изменить подпись своей сторис.
    async fn edit_story_caption(&self, _story_id: i32, _caption: &str) -> anyhow::Result<()> {
        anyhow::bail!("story edit is not implemented for this backend")
    }
    /// Назначить/снять админа с тонкими правами (нужны права владельца).
    /// Пустой набор прав = разжаловать.
    async fn set_admin(
        &self,
        _chat_id: i64,
        _user_id: i64,
        _access_hash: i64,
        _rights: AdminRights,
        _title: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("admin management is not implemented for this backend")
    }
    /// Слоумод чата (0 — выкл, иначе секунды задержки).
    async fn set_slowmode(&self, _chat_id: i64, _seconds: i32) -> anyhow::Result<()> {
        anyhow::bail!("slow mode is not implemented for this backend")
    }
    /// Подборки сторис (хайлайтс) текущего пользователя.
    async fn story_albums(&self) -> anyhow::Result<Vec<StoryAlbumInfo>> {
        Ok(vec![])
    }
    /// Создать подборку из своих сторис. Возвращает album_id.
    async fn create_album(&self, _title: &str, _story_ids: &[i32]) -> anyhow::Result<i32> {
        anyhow::bail!("story albums are not implemented for this backend")
    }
    /// Содержимое подборки (сторис).
    async fn album_stories(&self, _album_id: i32) -> anyhow::Result<Vec<StoryInfo>> {
        Ok(vec![])
    }
    /// Базовая аналитика канала.
    async fn channel_stats(&self, _chat_id: i64) -> anyhow::Result<ChannelStats> {
        anyhow::bail!("channel stats are not implemented for this backend")
    }
    /// Удалить подборку.
    async fn delete_album(&self, _album_id: i32) -> anyhow::Result<()> {
        anyhow::bail!("story albums are not implemented for this backend")
    }
    /// Кто голосовал в опросе (всего, имена). `option=None` — все варианты.
    async fn poll_voters(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _option: Option<Vec<u8>>,
    ) -> anyhow::Result<(i32, Vec<String>)> {
        Ok((0, vec![]))
    }
    /// Создать папку диалогов (id = max+1). Возвращает id.
    async fn create_folder(
        &self,
        _title: &str,
        _peer_ids: &[i64],
        _emoticon: Option<String>,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("folder management is not implemented for this backend")
    }
    /// Переименовать папку (состав и настройки сохраняются).
    async fn rename_folder(&self, _id: i32, _title: &str) -> anyhow::Result<()> {
        anyhow::bail!("folder management is not implemented for this backend")
    }
    /// Удалить папку диалогов.
    async fn delete_folder(&self, _id: i32) -> anyhow::Result<()> {
        anyhow::bail!("folder management is not implemented for this backend")
    }
    /// Перевести текст сообщения (Premium API сервера; без Premium — ошибка).
    /// Возвращает переведённый текст.
    async fn translate_message(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _to_lang: &str,
    ) -> anyhow::Result<String> {
        anyhow::bail!("translation is not implemented for this backend")
    }
    /// Репост чужой сторис к себе (скачивание → заливка → sendStory с fwd_from).
    /// Возвращает id новой сторис.
    async fn forward_story(
        &self,
        _peer_id: i64,
        _story_id: i32,
        _caption: &str,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("story repost is not implemented for this backend")
    }
    /// Эмодзи-статус аккаунта (document_id кастомного эмодзи; None — снять).
    async fn set_emoji_status(&self, _document_id: Option<i64>) -> anyhow::Result<()> {
        anyhow::bail!("emoji status is not implemented for this backend")
    }
    /// Дефолтные эмодзи-статусы (document_id).
    async fn default_emoji_statuses(&self) -> anyhow::Result<Vec<i64>> {
        Ok(vec![])
    }
    /// QR-вход, шаг 1: запросить токен. Возвращает (token_bytes, expires_unix).
    /// Токен кодируется как tg://login?token=<base64url(token)> и показывается QR.
    async fn qr_login_start(&self) -> anyhow::Result<(Vec<u8>, i64)> {
        anyhow::bail!("QR login is not implemented for this backend")
    }
    /// QR-вход, шаг 2: опросить статус токена. Ok(Some(user_id)) — вход выполнен.
    /// Ok(None) — ещё ждём сканирования; ошибка — токен истёк/отклонён, начать заново.
    async fn qr_login_poll(&self, _token: &[u8]) -> anyhow::Result<Option<i64>> {
        anyhow::bail!("QR login is not implemented for this backend")
    }
    /// Баланс звёзд Telegram Stars (только чтение, без трат).
    /// Возвращает (целые звёзды, нано-дробь).
    async fn stars_balance(&self) -> anyhow::Result<(i64, i32)> {
        Ok((0, 0))
    }
    /// Статистика канала: временной ряд просмотров/подписчиков для графиков.
    /// Возвращает (подписи оси X, значения) — пусто, если сервер отдал async/ошибку.
    async fn channel_graph(
        &self,
        _chat_id: i64,
        _kind: &str,
    ) -> anyhow::Result<(Vec<String>, Vec<f64>)> {
        Ok((vec![], vec![]))
    }
    /// Страница истории старше offset_id (для экспорта и прыжка к дате).
    /// Дефолт: берёт свежую историю и фильтрует — работает на любом бэкенде.
    async fn history_before(
        &self,
        chat_id: i64,
        offset_id: i32,
        limit: usize,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        let all = self.history(chat_id, limit.saturating_add(200)).await?;
        Ok(all
            .into_iter()
            .filter(|m| m.id < offset_id)
            .take(limit)
            .collect())
    }
    /// Окно истории вокруг даты (unix): листает history_before назад, пока не дойдёт.
    /// Возвращает (сообщения, достигнуто_ли_начало).
    async fn history_at_date(
        &self,
        chat_id: i64,
        date: i64,
        limit: usize,
    ) -> anyhow::Result<(Vec<MessageInfo>, bool)> {
        let mut cursor = i32::MAX;
        let mut window: Vec<MessageInfo> = Vec::new();
        for _ in 0..30 {
            let page = self.history_before(chat_id, cursor, 200).await?;
            if page.is_empty() {
                return Ok((window, true));
            }
            cursor = page.last().map(|m| m.id).unwrap_or(0);
            window = page;
            if window.iter().any(|m| m.date <= date) {
                break;
            }
        }
        let reached_start = window.is_empty();
        let out: Vec<MessageInfo> = window
            .into_iter()
            .filter(|m| (m.date - date).abs() < 86400 * 2)
            .take(limit)
            .collect();
        Ok((out, reached_start))
    }
    /// Завершить одну сессию по hash (account.resetAuthorization).
    async fn terminate_session(&self, _hash: i64) -> anyhow::Result<()> {
        anyhow::bail!("terminate_session is not implemented for this backend")
    }
    /// Автоудаление сообщений чата через N секунд, 0 — выключить (messages.setHistoryTTL).
    async fn set_history_ttl(&self, _chat_id: i64, _period_secs: i32) -> anyhow::Result<()> {
        anyhow::bail!("history TTL is not implemented for this backend")
    }
    /// Полное описание канала/группы (channels.getFullChannel).
    async fn chat_full(&self, _chat_id: i64) -> anyhow::Result<ChatFullInfo> {
        anyhow::bail!("chat_full is not implemented for this backend")
    }
    /// Полное описание собеседника: bio + флаги (users.getFullUser).
    async fn peer_full(&self, _chat_id: i64) -> anyhow::Result<PeerFullInfo> {
        anyhow::bail!("peer_full is not implemented for this backend")
    }
    /// Кто поставил реакции на сообщение (messages.getMessageReactionsList).
    async fn message_reactions(
        &self,
        _chat_id: i64,
        _message_id: i32,
    ) -> anyhow::Result<Vec<ReactionVoter>> {
        anyhow::bail!("message_reactions is not implemented for this backend")
    }
    /// Недавние стикеры с превью (messages.getRecentStickers).
    async fn recent_stickers(&self) -> anyhow::Result<Vec<StickerInfo>> {
        anyhow::bail!("recent_stickers is not implemented for this backend")
    }
    /// Отправить существующий документ (стикер/GIF) по ссылке на него.
    async fn send_document_ref(
        &self,
        _chat_id: i64,
        _doc_id: i64,
        _access_hash: i64,
        _file_reference: Vec<u8>,
        _caption: &str,
        _silent: bool,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("send_document_ref is not implemented for this backend")
    }
    /// Сохранённые GIF с превью (messages.getSavedGifs).
    async fn saved_gifs(&self) -> anyhow::Result<Vec<GifInfo>> {
        anyhow::bail!("saved_gifs is not implemented for this backend")
    }
    /// Отправить аудиофайл как голосовое (voice-атрибут + waveform).
    async fn send_voice(
        &self,
        _chat_id: i64,
        _path: &std::path::Path,
        _caption: &str,
        _silent: bool,
        _schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("voice sending is not implemented for this backend")
    }
    /// Люди и группы рядом (contacts.getLocated).
    async fn nearby(&self, _lat: f64, _long: f64) -> anyhow::Result<Vec<NearbyPeer>> {
        anyhow::bail!("nearby is not implemented for this backend")
    }
    /// Тарифы Stars-подарков для пользователя (только чтение).
    async fn star_gift_options(&self, _chat_id: i64) -> anyhow::Result<Vec<StarGiftOption>> {
        anyhow::bail!("star_gifts is not implemented for this backend")
    }
    /// Сменить/установить пароль 2FA (с проверкой нового через GetPasswordSettings).
    async fn change_password(
        &self,
        _current: &str,
        _new: &str,
        _hint: &str,
        _email: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("change_password is not implemented for this backend")
    }
    /// Каталог Stars-подарков (payments.getStarGifts).
    async fn gift_catalog(&self) -> anyhow::Result<Vec<StarGiftFull>> {
        anyhow::bail!("gifts are not implemented for this backend")
    }
    /// Отправить Stars-подарок (invoice → форма → sendStarsForm). Возвращает итог.
    async fn send_star_gift(
        &self,
        _chat_id: i64,
        _gift_id: i64,
        _hide_name: bool,
        _message: &str,
    ) -> anyhow::Result<String> {
        anyhow::bail!("gifts are not implemented for this backend")
    }
    /// Отправить видео как «кружок» (round_message, dims из первого кадра).
    async fn send_round_video(
        &self,
        _chat_id: i64,
        _path: &std::path::Path,
        _caption: &str,
        _silent: bool,
        _schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<i32> {
        anyhow::bail!("round video is not implemented for this backend")
    }
    /// Включить/выключить антиспам канала (channels.toggleAntiSpam).
    async fn toggle_antispam(&self, _chat_id: i64, _enabled: bool) -> anyhow::Result<()> {
        anyhow::bail!("antispam is not implemented for this backend")
    }
    /// Пожаловаться на спам от имени чата (messages.reportSpam).
    async fn report_spam(&self, _chat_id: i64) -> anyhow::Result<bool> {
        anyhow::bail!("report_spam is not implemented for this backend")
    }
    /// История Stars-транзакций (payments.getStarsTransactions).
    async fn stars_transactions(
        &self,
        _inbound: bool,
        _outbound: bool,
        _limit: i32,
    ) -> anyhow::Result<Vec<StarTxInfo>> {
        anyhow::bail!("stars_transactions is not implemented for this backend")
    }
    /// Мои сохранённые Stars-подарки (payments.getSavedStarGifts).
    async fn saved_star_gifts(&self) -> anyhow::Result<Vec<SavedGiftInfo>> {
        anyhow::bail!("saved_star_gifts is not implemented for this backend")
    }
    /// Подсказка пароля 2FA с сервера (account.getPassword.hint).
    async fn password_hint(&self) -> anyhow::Result<Option<String>> {
        anyhow::bail!("password_hint is not implemented for this backend")
    }
    fn is_connected(&self) -> bool;
}
