//! Реальный MTProto-бэкенд на `grammers`.
//!
//! Ключевое для ТЗ:
//! - Сторис читаются через `stories.getAllStories`.
//! - Скачивание из групп с `noforwards` идёт напрямую через
//!   `client.download_media(&media, path)` — сервер отдаёт байты, потому что
//!   флаг `noforwards` лишь прячет кнопки в официальных клиентах.

use super::{AccountInfo, ChatInfo, MessageInfo, StoryInfo, TelegramService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use grammers_session::types::PeerRef;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct GrammersBackend {
    client: Client,
    _session: Arc<SqliteSession>,
    api_id: i32,
    api_hash: String,
    /// Параллельных чанков MTProto (настраивается из Settings).
    parallel_workers: std::sync::Mutex<usize>,
}

// Grammers Client клонируемый и потокобезопасный (внутри Arc).
unsafe impl Send for GrammersBackend {}
unsafe impl Sync for GrammersBackend {}

impl GrammersBackend {
    pub async fn connect(session_path: &Path, api_id: i32, api_hash: &str) -> Result<Self> {
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let session = Arc::new(
            SqliteSession::open(session_path.to_string_lossy().to_string()).await?,
        );
        let SenderPool { runner, handle, .. } = SenderPool::new(Arc::clone(&session), api_id);
        let client = Client::new(handle);
        tokio::spawn(runner.run());
        Ok(Self {
            client,
            _session: session,
            api_id,
            api_hash: api_hash.to_string(),
            parallel_workers: std::sync::Mutex::new(
                crate::media::mtproto::DEFAULT_WORKERS,
            ),
        })
    }

    /// Число параллельных чанков для больших файлов (1..=8).
    pub fn set_parallel_workers(&self, n: usize) {
        *self.parallel_workers.lock().unwrap() = n.clamp(1, 8);
    }

    fn workers(&self) -> usize {
        *self.parallel_workers.lock().unwrap()
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn is_authorized(&self) -> Result<bool> {
        Ok(self.client.is_authorized().await?)
    }

    pub async fn request_code(&self, phone: &str) -> Result<grammers_client::client::LoginToken> {
        Ok(self.client.request_login_code(phone, &self.api_hash).await?)
    }

    pub async fn sign_in_code(
        &self,
        token: &grammers_client::client::LoginToken,
        code: &str,
    ) -> std::result::Result<(), SignInError> {
        self.client.sign_in(token, code).await.map(|_| ())
    }

    pub async fn sign_in_password(
        &self,
        token: grammers_client::client::PasswordToken,
        password: &str,
    ) -> Result<()> {
        self.client.check_password(token, password).await?;
        Ok(())
    }

    fn peer_protected(peer: &grammers_client::peer::Peer) -> bool {
        use grammers_client::peer::Peer;
        match peer {
            Peer::Channel(c) => c.raw.noforwards,
            Peer::Group(g) => {
                let dbg = format!("{:?}", g.raw);
                dbg.contains("noforwards: true")
            }
            Peer::Community(_) => false,
            Peer::User(_) => false,
        }
    }

    async fn resolve_ref(&self, bot_api_id: i64) -> Result<Option<PeerRef>> {
        use grammers_session::types::PeerId;
        let Some(pid) = PeerId::from_bot_api_dialog_id(bot_api_id) else {
            return Ok(None);
        };
        Ok(Some(pid.to_ambient_ref()))
    }

    /// Карта bot_api_id → unread_count через `messages.getPeerDialogs`
    /// (батчами по 100 — серверный лимит на размер запроса).
    /// Best-effort: при ошибках возвращает частичную карту.
    async fn peer_unread_map(
        &self,
        prefs: &[(i64, grammers_session::types::PeerRef)],
    ) -> Vec<(i64, i32)> {
        use grammers_client::tl as tl;
        let mut out = Vec::new();
        for chunk in prefs.chunks(100) {
            let inputs: Vec<tl::enums::InputDialogPeer> = chunk
                .iter()
                .map(|(_, pref)| {
                    tl::enums::InputDialogPeer::Peer(tl::types::InputDialogPeer {
                        peer: pref.into(),
                    })
                })
                .collect();
            let res = match self
                .client
                .invoke(&tl::functions::messages::GetPeerDialogs { peers: inputs })
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("getPeerDialogs failed: {e}");
                    continue;
                }
            };
            let dialogs = match res {
                tl::enums::messages::PeerDialogs::Dialogs(d) => d.dialogs,
            };
            for dlg in dialogs {
                let inner = match dlg {
                    tl::enums::Dialog::Dialog(d) => d,
                    _ => continue,
                };
                out.push((peer_to_bot_id(&inner.peer), inner.unread_count));
            }
        }
        out
    }
}

#[async_trait]
impl TelegramService for GrammersBackend {
    async fn dialogs(&self) -> Result<Vec<ChatInfo>> {
        use grammers_session::types::PeerRef;
        let mut out = Vec::new();
        let mut prefs: Vec<(i64, PeerRef)> = Vec::new();
        let mut iter = self.client.iter_dialogs();
        while let Some(d) = iter.next().await? {
            let peer = d.peer();
            let bot_id = d.peer_id().bot_api_dialog_id().unwrap_or(0);
            let title = peer.name().unwrap_or("(без названия)").to_string();
            let username = peer.username().map(|s| s.to_string());
            out.push(ChatInfo {
                id: bot_id,
                title,
                username,
                protected: Self::peer_protected(peer),
                unread: 0,
            });
            if let Some(pref) = self.resolve_ref(bot_id).await? {
                prefs.push((bot_id, pref));
            }
            if out.len() >= 300 {
                break;
            }
        }
        // Непрочитанные одним батчем (iter_dialogs счётчиков не даёт).
        // Best-effort: при ошибке остаются нули.
        for (bot_id, unread) in self.peer_unread_map(&prefs).await {
            if let Some(c) = out.iter_mut().find(|c| c.id == bot_id) {
                c.unread = unread;
            }
        }
        Ok(out)
    }

    async fn history(&self, chat_id: i64, limit: usize) -> Result<Vec<MessageInfo>> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut iter = self.client.iter_messages(pref).limit(limit.min(200));
        let mut out = Vec::new();
        while let Some(m) = iter.next().await? {
            let (kind, name, size) = match m.media() {
                None => (None, None, None),
                Some(media) => {
                    use grammers_client::media::Media as M;
                    match &media {
                        M::Photo(p) => (
                            Some("photo".to_string()),
                            Some(format!("photo_{}.jpg", m.id())),
                            p.size().map(|s| s as i64),
                        ),
                        M::Document(d) => {
                            let fname = d.name().map(|s| s.to_string());
                            let mime = d.mime_type().unwrap_or("");
                            let kind = if mime.starts_with("video") {
                                "video"
                            } else if mime.starts_with("audio") {
                                "audio"
                            } else {
                                "document"
                            };
                            (
                                Some(kind.to_string()),
                                fname.or_else(|| Some(format!("doc_{}", m.id()))),
                                d.size().map(|s| s as i64),
                            )
                        }
                        M::Sticker(_) => (
                            Some("sticker".to_string()),
                            Some(format!("sticker_{}.webp", m.id())),
                            media.size().map(|s| s as i64),
                        ),
                        _ => (
                            Some("media".to_string()),
                            Some(format!("media_{}", m.id())),
                            media.size().map(|s| s as i64),
                        ),
                    }
                }
            };
            out.push(MessageInfo {
                id: m.id(),
                chat_id,
                text: m.text().to_string(),
                date: m.date().as_second(),
                has_media: kind.is_some(),
                media_kind: kind,
                file_name: name,
                file_size: size,
                outgoing: m.outgoing(),
                // Имя отправителя — для групп/каналов (у своих тоже есть, UI решает).
                sender: m.sender().and_then(|p| p.name().map(|s| s.to_string())),
                edited: m.edit_date().is_some(),
                reactions: m.reaction_count(),
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    async fn stories(&self) -> Result<Vec<StoryInfo>> {
        use grammers_client::tl as tl;
        let mut out = Vec::new();
        let all = self
            .client
            .invoke(&tl::functions::stories::GetAllStories {
                next: false,
                hidden: false,
                state: None,
            })
            .await;
        let all = match all {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("stories.getAllStories failed: {e}; fallback to empty");
                return Ok(out);
            }
        };
        for (peer_bot_id, title, items) in parse_all_stories(&all) {
            for (story_id, caption, has_url) in items {
                out.push(StoryInfo {
                    peer_id: peer_bot_id,
                    peer_title: title.clone(),
                    story_id,
                    caption,
                    has_url,
                    date: 0,
                    preview_path: None,
                });
            }
            if out.len() >= 200 {
                break;
            }
        }
        Ok(out)
    }

    async fn download_message_media(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &Path,
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(dest_dir).await.ok();
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let msgs = self.client.get_messages_by_id(pref, &[message_id]).await?;
        let msg_opt = msgs.into_iter().next().flatten();
        let msg = msg_opt.context("message not found (deleted?)")?;
        let media = msg.media().context("message has no media")?;
        let fname = media_file_name(&media, message_id);
        let dest = dest_dir.join(&fname);
        // ВАЖНО: качаем напрямую через MTProto, игнорируя UI-флаг noforwards.
        self.client.download_media(&media, &dest).await?;
        Ok(dest)
    }

    async fn download_message_media_with_progress(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
        workers: usize,
        on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(dest_dir).await.ok();
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let msgs = self.client.get_messages_by_id(pref, &[message_id]).await?;
        let msg_opt = msgs.into_iter().next().flatten();
        let msg = msg_opt.context("message not found (deleted?)")?;
        let media = msg.media().context("message has no media")?;
        let fname = media_file_name(&media, message_id);
        let dest = dest_dir.join(&fname);
        // Параллельные чанки + resume + прогресс.
        // workers из конфига имеют приоритет над параметром, если заданы.
        let w = {
            let cfg_w = self.workers();
            if cfg_w != crate::media::mtproto::DEFAULT_WORKERS {
                cfg_w
            } else {
                workers.clamp(1, 8)
            }
        };
        crate::media::mtproto::download_parallel(&self.client, &media, &dest, w, on_progress)
            .await
    }

    async fn download_story_media(
        &self,
        peer_id: i64,
        story_id: i32,
        dest_dir: &Path,
    ) -> Result<PathBuf> {
        use grammers_client::tl as tl;
        tokio::fs::create_dir_all(dest_dir).await.ok();
        // Кэш: не качаем повторно.
        let cached_jpg = dest_dir.join(format!("story_{peer_id}_{story_id}.jpg"));
        let cached_png = dest_dir.join(format!("story_{peer_id}_{story_id}.png"));
        let cached_mp4 = dest_dir.join(format!("story_{peer_id}_{story_id}.mp4"));
        for c in [&cached_jpg, &cached_png, &cached_mp4] {
            if c.exists() {
                if let Ok(meta) = std::fs::metadata(c) {
                    if meta.len() > 0 {
                        return Ok(c.clone());
                    }
                }
            }
        }
        let pref = self.resolve_ref(peer_id).await?.context("peer not found")?;
        let input_peer: tl::enums::InputPeer = (&pref).into();
        let stories = self
            .client
            .invoke(&tl::functions::stories::GetStoriesById {
                peer: input_peer,
                id: vec![story_id],
            })
            .await
            .context("stories.getStoriesByID failed")?;
        let media_opt = extract_story_media(&stories);
        let media_enum = media_opt.context("story has no downloadable media (expired?)")?;
        let media = grammers_client::media::Media::from_raw(media_enum)
            .context("unsupported story media type")?;
        let ext = match &media {
            grammers_client::media::Media::Photo(_) => "jpg",
            grammers_client::media::Media::Document(d) => {
                let mime = d.mime_type().unwrap_or("");
                if mime.contains("mp4") || mime.contains("video") {
                    "mp4"
                } else if mime.contains("jpg") || mime.contains("jpeg") {
                    "jpg"
                } else if mime.contains("png") {
                    "png"
                } else {
                    "jpg"
                }
            }
            _ => "jpg",
        };
        let dest = dest_dir.join(format!("story_{peer_id}_{story_id}.{ext}"));
        // Встроенный video-preview без ffmpeg: тамб Telegram (первый кадр,
        // нарезанный сервером) — качаем как poster.jpg рядом с видео.
        // Дальше цепочка в UI: poster → H.264/HEVC/AV1 → embedded cover (mp4) → ffmpeg → плейсхолдер.
        if ext == "mp4" {
            if let grammers_client::media::Media::Document(d) = &media {
                if let Some(thumb) = largest_thumb(&d.thumbs()) {
                    let poster = dest_dir.join(format!("story_{peer_id}_{story_id}.poster.jpg"));
                    if !crate::media::video::poster_fresh(&dest, &poster) {
                        match self.client.download_media(&thumb, &poster).await {
                            Ok(_) => tracing::debug!("story {peer_id}:{story_id} thumb ok"),
                            Err(e) => tracing::debug!("story thumb failed: {e}"),
                        }
                    }
                }
            }
        }
        self.client.download_media(&media, &dest).await?;
        Ok(dest)
    }

    async fn me(&self) -> Result<AccountInfo> {
        let me = self.client.get_me().await?;
        let name = me.full_name();
        let display = if name.trim().is_empty() {
            me.username().unwrap_or("(я)").to_string()
        } else {
            name
        };
        Ok(AccountInfo {
            id: me.id().bot_api_dialog_id().unwrap_or(0),
            name: display,
            phone: me.phone().map(|s| s.to_string()),
            username: me.username().map(|s| s.to_string()),
        })
    }

    async fn send_text(&self, chat_id: i64, text: &str, reply_to: Option<i32>) -> Result<i32> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let sent = self
            .client
            .send_message(pref, InputMessage::new().text(text).reply_to(reply_to))
            .await
            .context("send_message failed")?;
        Ok(sent.id())
    }

    async fn edit_text(&self, chat_id: i64, message_id: i32, text: &str) -> Result<()> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .edit_message(pref, message_id, InputMessage::new().text(text))
            .await
            .context("edit_message failed")?;
        Ok(())
    }

    async fn delete_messages(&self, chat_id: i64, message_ids: &[i32]) -> Result<()> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .delete_messages(pref, message_ids)
            .await
            .context("delete_messages failed")?;
        Ok(())
    }

    async fn react(&self, chat_id: i64, message_id: i32, emoji: &str) -> Result<()> {
        use grammers_client::message::InputReactions;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let msgs = self.client.get_messages_by_id(pref, &[message_id]).await?;
        let msg = msgs
            .into_iter()
            .next()
            .flatten()
            .context("message not found (deleted?)")?;
        // Повтор той же реакции сервером переключается (toggle) — убирается.
        msg.react(InputReactions::emoticon(emoji))
            .await
            .context("sendReaction failed")?;
        Ok(())
    }

    async fn mark_read(&self, chat_id: i64) -> Result<()> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // Best-effort: счётчик в UI всё равно обновится следующим refresh_dialogs.
        if let Err(e) = self.client.mark_as_read(pref).await {
            tracing::debug!("mark_as_read failed: {e}");
        }
        Ok(())
    }

    async fn send_file(&self, chat_id: i64, path: &Path, caption: &str) -> Result<i32> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let uploaded = self.client.upload_file(path).await.context("upload_file failed")?;
        let is_photo = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
            .unwrap_or(false);
        let input = if is_photo {
            InputMessage::new().text(caption).photo(uploaded)
        } else {
            InputMessage::new().text(caption).document(uploaded)
        };
        let sent = self.client.send_message(pref, input).await.context("send file failed")?;
        Ok(sent.id())
    }

    async fn avatar_path(&self, chat_id: i64) -> Result<Option<PathBuf>> {
        let dir = super::avatars_cache_dir();
        std::fs::create_dir_all(&dir).ok();
        let dest = dir.join(format!("{chat_id}.jpg"));
        // Кэш: не качаем повторно (аватар меняется редко; stale чистится удалением файла).
        if let Ok(meta) = std::fs::metadata(&dest) {
            if meta.len() > 0 {
                return Ok(Some(dest));
            }
        }
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut iter = self.client.iter_profile_photos(pref);
        let photo = match iter.next().await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("profile photos failed: {e}");
                return Ok(None);
            }
        };
        let photo = match photo {
            Some(p) => p,
            None => return Ok(None),
        };
        let media = grammers_client::media::Media::Photo(photo);
        match self.client.download_media(&media, &dest).await {
            Ok(_) => {
                if std::fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false) {
                    Ok(Some(dest))
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                tracing::debug!("avatar download failed: {e}");
                Ok(None)
            }
        }
    }

    fn is_connected(&self) -> bool {
        true
    }
}

fn media_file_name(media: &grammers_client::media::Media, message_id: i32) -> String {
    use grammers_client::media::Media as M;
    match media {
        M::Photo(_) => format!("photo_{message_id}.jpg"),
        M::Document(d) => d
            .name()
            .map(sanitize_filename)
            .unwrap_or_else(|| format!("doc_{message_id}.bin")),
        M::Sticker(_) => format!("sticker_{message_id}.webp"),
        _ => format!("media_{message_id}.bin"),
    }
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}

fn parse_all_stories(
    all: &grammers_client::tl::enums::stories::AllStories,
) -> Vec<(i64, String, Vec<(i32, String, bool)>)> {
    use grammers_client::tl::enums::stories::AllStories as A;
    let mut result = Vec::new();
    let inner: &grammers_client::tl::types::stories::AllStories = match all {
        A::Stories(all) => all,
        A::NotModified(_) => return result,
    };
    for ps in &inner.peer_stories {
        // enums::PeerStories::Stories(types::PeerStories)
        let (peer, stories) = match ps {
            grammers_client::tl::enums::PeerStories::Stories(ps) => (&ps.peer, &ps.stories),
        };
        let bot_id = peer_to_bot_id(peer);
        let title = format!("peer {bot_id}");
        let mut items = Vec::new();
        for st in stories {
            match st {
                grammers_client::tl::enums::StoryItem::Item(full) => {
                    let caption = full.caption.clone().unwrap_or_default();
                    let has_url = caption.contains("http")
                        || caption.contains("t.me")
                        || full
                            .entities
                            .as_ref()
                            .map(|ents| {
                                ents.iter().any(|e| {
                                    let d = format!("{e:?}");
                                    d.contains("Url") || d.contains("TextUrl")
                                })
                            })
                            .unwrap_or(false);
                    items.push((full.id, caption, has_url));
                }
                grammers_client::tl::enums::StoryItem::Skipped(s) => {
                    let _ = s.id;
                }
                grammers_client::tl::enums::StoryItem::Deleted(d) => {
                    let _ = d.id;
                }
            }
        }
        if !items.is_empty() {
            result.push((bot_id, title, items));
        }
    }
    result
}

fn peer_to_bot_id(peer: &grammers_client::tl::enums::Peer) -> i64 {
    use grammers_client::tl::enums::Peer as P;
    match peer {
        P::User(u) => u.user_id,
        P::Chat(c) => -c.chat_id,
        P::Channel(c) => -(1_000_000_000_000 + c.channel_id),
    }
}

/// Самый большой скачиваемый тамб (PhotoSize::Size) для video-preview.
fn largest_thumb(thumbs: &[grammers_client::media::PhotoSize]) -> Option<grammers_client::media::PhotoSize> {
    use grammers_client::media::PhotoSize as PS;
    thumbs
        .iter()
        .filter_map(|t| match t {
            PS::Size(s) => Some((s.width as i64 * s.height as i64, t.clone())),
            _ => None,
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, t)| t)
}

/// Достать `MessageMedia` из ответа `stories.getStoriesByID`.
fn extract_story_media(
    stories: &grammers_client::tl::enums::stories::Stories,
) -> Option<grammers_client::tl::enums::MessageMedia> {
    use grammers_client::tl::enums::stories::Stories as S;
    let inner = match stories {
        S::Stories(s) => s,
    };
    for st in &inner.stories {
        match st {
            grammers_client::tl::enums::StoryItem::Item(full) => {
                return Some(full.media.clone());
            }
            _ => continue,
        }
    }
    None
}
