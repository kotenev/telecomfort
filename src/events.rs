//! События фона → UI (`BgEvent`) и подтверждения опасных действий.
//!
//! Вынесены из `ui::app`, чтобы фоновый код (`media`, `sync`, Android-мост)
//! ссылался на них без eframe.

use std::path::PathBuf;

use crate::telegram::{AccountInfo, ChatInfo, MessageInfo, StoryInfo};

#[derive(Debug, Clone)]
pub enum BgEvent {
    Dialogs(Vec<ChatInfo>),
    History {
        chat_id: i64,
        items: Vec<MessageInfo>,
    },
    Stories(Vec<StoryInfo>),
    StoryPreview {
        peer_id: i64,
        story_id: i32,
        path: PathBuf,
    },
    /// Готовый постер первого кадра видеосторис.
    StoryPoster {
        peer_id: i64,
        story_id: i32,
        poster: PathBuf,
    },
    /// Прогресс заливки на Яндекс Диск (стриминг чанками).
    UploadProgress {
        local_path: String,
        done: u64,
        total: u64,
    },
    /// Прогресс скачивания из MTProto (параллельные чанки + resume).
    MediaProgress {
        label: String,
        done: u64,
        total: u64,
    },
    /// Своё отправленное сообщение (серверный id) — добавить в историю.
    SentMessage {
        chat_id: i64,
        item: crate::telegram::MessageInfo,
    },
    /// Своя правка применена — обновить текст локально.
    EditedMessage {
        chat_id: i64,
        message_id: i32,
        text: String,
    },
    /// Удаление применено — выкинуть локально.
    DeletedMessages {
        chat_id: i64,
        message_ids: Vec<i32>,
    },
    /// Перезагрузить историю чата (после реакций/чужих действий).
    ReloadHistory {
        chat_id: i64,
    },
    /// Аватар скачался — запомнить файл, сбросить текстуру-идентикон.
    AvatarReady {
        chat_id: i64,
        path: PathBuf,
    },
    /// Закреплённое сообщение чата (None — нет закрепа).
    PinnedLoaded {
        chat_id: i64,
        item: Option<crate::telegram::MessageInfo>,
    },
    /// Папки диалогов сервера.
    FoldersLoaded(Vec<crate::telegram::DialogFilterInfo>),
    /// Контакты телефонной книги.
    ContactsLoaded(Vec<crate::telegram::ContactInfo>),
    /// Перезагрузить список диалогов (после мьют/архив).
    ReloadDialogs,
    /// Аудио скачалось в кэш — начать воспроизведение.
    AudioReady {
        chat_id: i64,
        message_id: i32,
        path: PathBuf,
    },
    /// Пересылка выполнена.
    Forwarded {
        from_chat: i64,
        to_chat: i64,
        count: usize,
    },
    /// Стикер скачался в кэш — показать превью в пузыре.
    StickerReady {
        message_id: i32,
        path: PathBuf,
    },
    /// Активные сессии загружены.
    SessionsLoaded(Vec<crate::telegram::SessionInfo>),
    /// Профиль загружен.
    ProfileLoaded(crate::telegram::ProfileInfo),
    /// Сторис опубликована.
    StoryPublished {
        story_id: i32,
    },
    /// Топики форума чата.
    TopicsLoaded {
        chat_id: i64,
        topics: Vec<crate::telegram::TopicInfo>,
    },
    /// История топика (заменяет общую историю в UI).
    TopicHistoryLoaded {
        chat_id: i64,
        topic_id: i32,
        items: Vec<crate::telegram::MessageInfo>,
    },
    /// Создан новый топик.
    TopicCreated {
        chat_id: i64,
        topic_id: i32,
    },
    /// Результаты глобального поиска.
    GlobalSearchResults(Vec<(i64, crate::telegram::MessageInfo)>),
    /// Запланированные сообщения чата.
    ScheduledLoaded {
        chat_id: i64,
        items: Vec<crate::telegram::MessageInfo>,
    },
    /// Инвайт-ссылка готова.
    InviteLinkReady(String),
    /// Видео-превью скачалось в кэш.
    VideoThumbReady {
        message_id: i32,
        path: PathBuf,
    },
    /// Фото веб-превью скачалось в кэш.
    WebThumbReady {
        photo_id: i64,
        path: PathBuf,
    },
    /// Канал/группа созданы.
    ChannelCreated {
        chat_id: i64,
    },
    /// Сторис удалены — перезагрузить ленту.
    StoriesDeleted,
    /// Опрос создан.
    PollCreated {
        chat_id: i64,
        message_id: i32,
    },
    /// Геопозиция отправлена.
    GeoSent {
        chat_id: i64,
        message_id: i32,
    },
    /// Ответ на сторис отправлен.
    StoryReplySent {
        story_id: i32,
        message_id: i32,
    },
    /// Просмотры сторис загружены.
    StoryViewsLoaded {
        story_id: i32,
        views: i32,
    },
    /// Стелс-режим включён.
    StealthEnabled,
    /// Зрители сторис загружены.
    StoryViewersLoaded {
        story_id: i32,
        count: i32,
        viewers: Vec<String>,
    },
    /// Медиа-таб истории загружен.
    MediaTabLoaded {
        chat_id: i64,
        items: Vec<crate::telegram::MessageInfo>,
    },
    /// Чёрный список загружен.
    BlockedLoaded(Vec<crate::telegram::BlockedInfo>),
    /// Результат проверки username.
    UsernameChecked(bool),
    /// Закреп диалога переключён.
    DialogPinToggled {
        chat_id: i64,
        pinned: bool,
    },
    /// Перевод сообщения готов.
    TranslatedMessage {
        chat_id: i64,
        message_id: i32,
        text: String,
    },
    /// Репост сторис выполнен.
    StoryForwarded {
        story_id: i32,
    },
    /// Права администратора применены.
    AdminSaved {
        chat_id: i64,
    },
    /// Эмодзи-статусы загружены.
    EmojiStatusesLoaded(Vec<i64>),
    /// Свой эмодзи-статус изменён.
    OwnEmojiStatus(Option<i64>),
    /// Альбом файлов отправлен.
    AlbumSent {
        chat_id: i64,
        count: usize,
    },
    /// Архив сторис загружен.
    ArchiveLoaded(Vec<crate::telegram::StoryInfo>),
    /// Подборки загружены.
    AlbumsLoaded(Vec<crate::telegram::StoryAlbumInfo>),
    /// Содержимое подборки загружено.
    AlbumStoriesLoaded {
        album_id: i32,
        items: Vec<crate::telegram::StoryInfo>,
    },
    /// Аналитика канала загружена.
    ChannelStatsLoaded {
        chat_id: i64,
        stats: crate::telegram::ChannelStats,
    },
    /// Архивное превью готово.
    ArchiveThumbReady {
        story_id: i32,
        path: PathBuf,
    },
    /// Подборка создана/удалена.
    AlbumChanged,
    /// Голоса загружены.
    PollVotersLoaded {
        message_id: i32,
        count: i32,
        voters: Vec<String>,
    },
    /// Папки перезагрузить.
    FoldersChanged,
    /// Контакт расшарен.
    ShareContactSent {
        chat_id: i64,
        message_id: i32,
    },
    /// Опрос остановлен.
    PollStopped {
        chat_id: i64,
    },
    /// Все закрепы загружены.
    PinnedListLoaded {
        chat_id: i64,
        items: Vec<crate::telegram::MessageInfo>,
    },
    /// Инвайт отозван.
    InviteRevoked,
    /// Участники загружены.
    ParticipantsLoaded {
        chat_id: i64,
        items: Vec<crate::telegram::ParticipantInfo>,
    },
    /// Подпись сторис обновлена.
    StoryCaptionSaved {
        story_id: i32,
    },
    /// Перезагрузить ленту сторис.
    ReloadStories,
    /// QR-токен получен — показать код.
    QrToken {
        token: Vec<u8>,
        expires: i64,
    },
    /// QR подтверждён на телефоне — войти.
    QrAccepted,
    /// QR протух или отменён.
    QrExpired(String),
    /// Баланс звёзд загружен.
    StarsLoaded((i64, i32)),
    /// Временной ряд графика готов.
    GraphLoaded {
        chat_id: i64,
        kind: String,
        labels: Vec<String>,
        values: Vec<f64>,
    },
    Me(AccountInfo),
    /// Экспорт истории записан в файл.
    HistoryExported {
        chat_id: i64,
        path: String,
    },
    /// Полное описание чата загружено.
    ChatFullLoaded {
        chat_id: i64,
        info: crate::telegram::ChatFullInfo,
    },
    /// Полное описание собеседника загружено.
    PeerFullLoaded {
        chat_id: i64,
        info: crate::telegram::PeerFullInfo,
    },
    /// Список проголосовавших реакцией.
    VotersLoaded {
        chat_id: i64,
        message_id: i32,
        voters: Vec<crate::telegram::ReactionVoter>,
    },
    /// Недавние стикеры готовы.
    StickersLoaded(Vec<crate::telegram::StickerInfo>),
    /// Сохранённые GIF готовы.
    GifsLoaded(Vec<crate::telegram::GifInfo>),
    /// Рядом находящиеся пиры.
    NearbyLoaded(Vec<crate::telegram::NearbyPeer>),
    /// Тарифы Stars-подарков.
    StarGiftsLoaded {
        chat_id: i64,
        options: Vec<crate::telegram::StarGiftOption>,
    },
    /// Каталог подарков для отправки.
    GiftCatalogLoaded(Vec<crate::telegram::StarGiftFull>),
    /// История Stars-транзакций.
    StarsTxLoaded(Vec<crate::telegram::StarTxInfo>),
    /// Мои сохранённые подарки.
    SavedGiftsLoaded(Vec<crate::telegram::SavedGiftInfo>),
    /// Подсказка пароля 2FA.
    PwHintLoaded(Option<String>),
    /// Прыжок к дате: окно истории + достигнуто ли начало.
    JumpLoaded {
        chat_id: i64,
        items: Vec<crate::telegram::MessageInfo>,
        reached_start: bool,
    },
    Log(String),
    Error(String),
    Connected {
        backend_desc: String,
    },
    CodeSent,
    SignedIn,
    SyncTick,
    SyncDone {
        stories: usize,
        downloads: usize,
    },
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Очистить историю чата (revoke — и у всех).
    ClearHistory { chat_id: i64, revoke: bool },
    /// Покинуть канал/супергруппу.
    LeaveChannel { chat_id: i64 },
    /// Удалить свои сторис.
    DeleteStories { story_ids: Vec<i32> },
    /// Пожаловаться на спам от имени чата.
    ReportSpam { chat_id: i64 },
}
