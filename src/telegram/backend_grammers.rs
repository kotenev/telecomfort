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

    /// Карта bot_api_id → (unread, muted, archived) через `messages.getPeerDialogs`
    /// (батчами по 100 — серверный лимит на размер запроса).
    /// Best-effort: при ошибках возвращает частичную карту.
    async fn peer_unread_map(
        &self,
        prefs: &[(i64, grammers_session::types::PeerRef)],
    ) -> Vec<(i64, i32, bool, bool)> {
        use grammers_client::tl as tl;
        // mute_until сравниваем с текущим unix-временем.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i32)
            .unwrap_or(0);
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
                let muted = match &inner.notify_settings {
                    tl::enums::PeerNotifySettings::Settings(s) => {
                        s.mute_until.map(|t| t > now).unwrap_or(false)
                    }
                    _ => false,
                };
                let archived = inner.folder_id == Some(1);
                out.push((peer_to_bot_id(&inner.peer), inner.unread_count, muted, archived));
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
                muted: false,
                archived: false,
            });
            if let Some(pref) = self.resolve_ref(bot_id).await? {
                prefs.push((bot_id, pref));
            }
            if out.len() >= 300 {
                break;
            }
        }
        // Непрочитанные/мьют/архив одним батчем (iter_dialogs счётчиков не даёт).
        // Best-effort: при ошибке остаются нули/false.
        for (bot_id, unread, muted, archived) in self.peer_unread_map(&prefs).await {
            if let Some(c) = out.iter_mut().find(|c| c.id == bot_id) {
                c.unread = unread;
                c.muted = muted;
                c.archived = archived;
            }
        }
        Ok(out)
    }

    async fn history(&self, chat_id: i64, limit: usize) -> Result<Vec<MessageInfo>> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut iter = self.client.iter_messages(pref).limit(limit.min(200));
        let mut out = Vec::new();
        while let Some(m) = iter.next().await? {
            out.push(message_to_info(&m, chat_id));
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    async fn pinned_message(&self, chat_id: i64) -> Result<Option<MessageInfo>> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut iter = self
            .client
            .search_messages(pref)
            .filter(tl::enums::MessagesFilter::InputMessagesFilterPinned)
            .limit(1);
        let msg = match iter.next().await {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("pinned search failed: {e}");
                return Ok(None);
            }
        };
        Ok(msg.map(|m| message_to_info(&m, chat_id)))
    }

    async fn pin_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::UpdatePinnedMessage {
                silent: true,
                unpin: false,
                pm_oneside: false,
                peer: (&pref).into(),
                id: message_id,
            })
            .await
            .context("pin failed (no rights?)")?;
        Ok(())
    }

    async fn unpin_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::UpdatePinnedMessage {
                silent: true,
                unpin: true,
                pm_oneside: false,
                peer: (&pref).into(),
                id: message_id,
            })
            .await
            .context("unpin failed")?;
        Ok(())
    }

    async fn vote(&self, chat_id: i64, message_id: i32, option: Vec<u8>) -> Result<()> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::SendVote {
                peer: (&pref).into(),
                msg_id: message_id,
                options: vec![option],
            })
            .await
            .context("vote failed")?;
        Ok(())
    }

    async fn forward(&self, from_chat: i64, message_ids: &[i32], to_chat: i64) -> Result<usize> {
        let from = self.resolve_ref(from_chat).await?.context("source peer not found")?;
        let to = self.resolve_ref(to_chat).await?.context("target peer not found")?;
        let sent = self
            .client
            .forward_messages(to, message_ids, from)
            .await
            .context("forward failed")?;
        Ok(sent.into_iter().flatten().count())
    }

    async fn mute(&self, chat_id: i64, muted: bool) -> Result<()> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // mute_until = i32::MAX (навсегда) либо 0 (снять). Остальные поля — None (не трогаем).
        let settings = tl::types::InputPeerNotifySettings {
            show_previews: None,
            silent: None,
            mute_until: Some(if muted { i32::MAX } else { 0 }),
            sound: None,
            stories_muted: None,
            stories_hide_sender: None,
            stories_sound: None,
        };
        self.client
            .invoke(&tl::functions::account::UpdateNotifySettings {
                peer: tl::enums::InputNotifyPeer::Peer(tl::types::InputNotifyPeer {
                    peer: (&pref).into(),
                }),
                settings: tl::enums::InputPeerNotifySettings::Settings(settings),
            })
            .await
            .context("mute failed")?;
        Ok(())
    }

    async fn archive(&self, chat_id: i64, archived: bool) -> Result<()> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::folders::EditPeerFolders {
                folder_peers: vec![tl::enums::InputFolderPeer::Peer(tl::types::InputFolderPeer {
                    peer: (&pref).into(),
                    folder_id: if archived { 1 } else { 0 },
                })],
            })
            .await
            .context("archive failed")?;
        Ok(())
    }

    async fn dialog_filters(&self) -> Result<Vec<super::DialogFilterInfo>> {
        use grammers_client::tl as tl;
        let res = self
            .client
            .invoke(&tl::functions::messages::GetDialogFilters {})
            .await
            .context("getDialogFilters failed")?;
        let filters = match res {
            tl::enums::messages::DialogFilters::Filters(f) => f.filters,
        };
        let mut out = Vec::new();
        for f in filters {
            let df = match f {
                tl::enums::DialogFilter::Filter(df) => df,
                _ => continue,
            };
            let title = match &df.title {
                tl::enums::TextWithEntities::Entities(t) => t.text.clone(),
            };
            let mut peers: Vec<i64> = df
                .pinned_peers
                .iter()
                .chain(df.include_peers.iter())
                .filter_map(input_peer_to_bot_id)
                .collect();
            peers.sort_unstable();
            peers.dedup();
            if title.is_empty() || peers.is_empty() {
                continue;
            }
            out.push(super::DialogFilterInfo { id: df.id, title, peer_ids: peers });
        }
        Ok(out)
    }

    async fn contacts(&self) -> Result<Vec<super::ContactInfo>> {
        use grammers_client::tl as tl;
        let res = self
            .client
            .invoke(&tl::functions::contacts::GetContacts { hash: 0 })
            .await
            .context("getContacts failed")?;
        let users = match res {
            tl::enums::contacts::Contacts::Contacts(c) => c.users,
            _ => return Ok(vec![]),
        };
        let mut out = Vec::new();
        for u in users {
            let user = match u {
                tl::enums::User::User(u) => u,
                _ => continue,
            };
            let name = match (&user.first_name, &user.last_name) {
                (Some(f), Some(l)) => format!("{f} {l}"),
                (Some(f), None) => f.clone(),
                (None, Some(l)) => l.clone(),
                (None, None) => user.username.clone().unwrap_or_else(|| format!("id{}", user.id)),
            };
            out.push(super::ContactInfo {
                name,
                phone: user.phone.clone().unwrap_or_default(),
                username: user.username.clone(),
                user_id: user.id,
            });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
            .send_message(
                pref,
                InputMessage::new().text(text).reply_to(reply_to).clear_draft(true),
            )
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
            InputMessage::new().text(caption).photo(uploaded).clear_draft(true)
        } else {
            InputMessage::new().text(caption).document(uploaded).clear_draft(true)
        };
        let sent = self.client.send_message(pref, input).await.context("send file failed")?;
        Ok(sent.id())
    }

    async fn send_scheduled(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i32>,
        at: std::time::SystemTime,
    ) -> Result<i32> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let sent = self
            .client
            .send_message(
                pref,
                InputMessage::new()
                    .text(text)
                    .reply_to(reply_to)
                    .schedule_date(Some(at))
                    .clear_draft(true),
            )
            .await
            .context("scheduled send failed")?;
        Ok(sent.id())
    }

    async fn save_draft(&self, chat_id: i64, text: &str) -> Result<()> {
        use grammers_client::tl as tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // Пустой текст — сервер стирает черновик (синк между устройствами).
        self.client
            .invoke(&tl::functions::messages::SaveDraft {
                no_webpage: true,
                invert_media: false,
                reply_to: None,
                peer: (&pref).into(),
                message: text.to_string(),
                entities: None,
                media: None,
                effect: None,
                rich_message: None,
                suggested_post: None,
            })
            .await
            .context("saveDraft failed")?;
        Ok(())
    }

    async fn sessions(&self) -> Result<Vec<super::SessionInfo>> {
        use grammers_client::tl as tl;
        let res = self
            .client
            .invoke(&tl::functions::account::GetAuthorizations {})
            .await
            .context("getAuthorizations failed")?;
        let list = match res {
            tl::enums::account::Authorizations::Authorizations(a) => a.authorizations,
        };
        Ok(list
            .into_iter()
            .filter_map(|a| match a {
                tl::enums::Authorization::Authorization(a) => Some(super::SessionInfo {
                    current: a.current,
                    app_name: a.app_name,
                    device: if a.device_model.is_empty() {
                        a.platform.clone()
                    } else {
                        format!("{} ({})", a.device_model, a.platform)
                    },
                    platform: a.system_version,
                    country: if a.country.is_empty() { a.ip.clone() } else { a.country },
                    ip: a.ip,
                    last_active: a.date_active as i64,
                    hash: a.hash,
                }),
                _ => None,
            })
            .collect())
    }

    async fn terminate_others(&self) -> Result<()> {
        use grammers_client::tl as tl;
        self.client
            .invoke(&tl::functions::auth::ResetAuthorizations {})
            .await
            .context("resetAuthorizations failed")?;
        Ok(())
    }

    async fn get_profile(&self) -> Result<super::ProfileInfo> {
        use grammers_client::tl as tl;
        let me = self.client.get_me().await?;
        let res = self
            .client
            .invoke(&tl::functions::users::GetFullUser {
                id: tl::enums::InputUser::UserSelf,
            })
            .await
            .context("getFullUser failed")?;
        let about = match res {
            tl::enums::users::UserFull::Full(f) => match f.full_user {
                tl::enums::UserFull::Full(u) => u.about.clone().unwrap_or_default(),
                _ => String::new(),
            },
        };
        Ok(super::ProfileInfo {
            first_name: me.first_name().unwrap_or("").to_string(),
            last_name: me.last_name().unwrap_or("").to_string(),
            about,
        })
    }

    async fn update_profile(&self, first_name: &str, last_name: &str, about: &str) -> Result<()> {
        use grammers_client::tl as tl;
        self.client
            .invoke(&tl::functions::account::UpdateProfile {
                first_name: Some(first_name.to_string()),
                last_name: Some(last_name.to_string()),
                about: Some(about.to_string()),
            })
            .await
            .context("updateProfile failed")?;
        Ok(())
    }

    async fn publish_story(&self, path: &Path, caption: &str) -> Result<i32> {
        use grammers_client::tl as tl;
        let uploaded =
            self.client.upload_file(path).await.context("story upload failed")?;
        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "story.bin".to_string());
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        let is_photo = matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp");
        let upload_media: tl::enums::InputMedia = if is_photo {
            tl::enums::InputMedia::UploadedPhoto(tl::types::InputMediaUploadedPhoto {
                spoiler: false,
                live_photo: false,
                file: uploaded.raw,
                stickers: None,
                ttl_seconds: None,
                video: None,
            })
        } else {
            // Размеры — из встроенного декодера первого кадра (без ffmpeg).
            let (w, h) = crate::media::video::first_frame_builtin(path)
                .map(|f| (f.width() as i32, f.height() as i32))
                .unwrap_or((0, 0));
            tl::enums::InputMedia::UploadedDocument(tl::types::InputMediaUploadedDocument {
                nosound_video: false,
                force_file: false,
                spoiler: false,
                file: uploaded.raw,
                thumb: None,
                mime_type: "video/mp4".to_string(),
                attributes: vec![
                    tl::enums::DocumentAttribute::Filename(
                        tl::types::DocumentAttributeFilename { file_name: fname },
                    ),
                    tl::enums::DocumentAttribute::Video(tl::types::DocumentAttributeVideo {
                        round_message: false,
                        supports_streaming: true,
                        nosound: false,
                        duration: 0.0,
                        w,
                        h,
                        preload_prefix_size: None,
                        video_start_ts: None,
                        video_codec: None,
                    }),
                ],
                stickers: None,
                video_cover: None,
                video_timestamp: None,
                ttl_seconds: None,
            })
        };
        // Шаг 2: messages.uploadMedia — сервер возвращает photo/document с id.
        let me_pref = self.resolve_ref(
            self.client.get_me().await?.id().bot_api_dialog_id().unwrap_or(0),
        ).await?.context("me peer not found")?;
        let uploaded_media = self
            .client
            .invoke(&tl::functions::messages::UploadMedia {
                business_connection_id: None,
                peer: (&me_pref).into(),
                media: upload_media,
            })
            .await
            .context("uploadMedia failed")?;
        let media: tl::enums::InputMedia = match uploaded_media {
            tl::enums::MessageMedia::Photo(p) => {
                let photo = p.photo.context("uploadMedia: no photo")?;
                let (id, access_hash, file_reference) = match photo {
                    tl::enums::Photo::Photo(p) => (p.id, p.access_hash, p.file_reference),
                    _ => anyhow::bail!("uploadMedia: unsupported photo"),
                };
                tl::enums::InputMedia::Photo(tl::types::InputMediaPhoto {
                    spoiler: false,
                    live_photo: false,
                    id: tl::enums::InputPhoto::Photo(tl::types::InputPhoto {
                        id,
                        access_hash,
                        file_reference,
                    }),
                    ttl_seconds: None,
                    video: None,
                })
            }
            tl::enums::MessageMedia::Document(d) => {
                let doc = d.document.context("uploadMedia: no document")?;
                let (id, access_hash, file_reference) = match doc {
                    tl::enums::Document::Document(d) => (d.id, d.access_hash, d.file_reference),
                    _ => anyhow::bail!("uploadMedia: unsupported document"),
                };
                tl::enums::InputMedia::Document(tl::types::InputMediaDocument {
                    spoiler: false,
                    id: tl::enums::InputDocument::Document(tl::types::InputDocument {
                        id,
                        access_hash,
                        file_reference,
                    }),
                    video_cover: None,
                    video_timestamp: None,
                    ttl_seconds: None,
                    query: None,
                })
            }
            _ => anyhow::bail!("uploadMedia: неожиданный тип медиа"),
        };
        // Шаг 3: stories.sendStory себе (видно контактам — как дефолт официального клиента).
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::stories::SendStory {
                pinned: false,
                noforwards: false,
                fwd_modified: false,
                peer: tl::enums::InputPeer::PeerSelf,
                media,
                media_areas: None,
                caption: if caption.is_empty() { None } else { Some(caption.to_string()) },
                entities: None,
                privacy_rules: vec![tl::enums::InputPrivacyRule::InputPrivacyValueAllowContacts],
                random_id,
                period: None,
                fwd_from_id: None,
                fwd_from_story: None,
                albums: None,
                music: None,
            })
            .await
            .context("sendStory failed")?;
        // Ответ — Updates: ищем UpdateStory с id новой сторис.
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => vec![],
        };
        for u in &updates {
            if let tl::enums::Update::Story(s) = u {
                if let tl::enums::StoryItem::Item(full) = &s.story {
                    return Ok(full.id);
                }
            }
        }
        anyhow::bail!("sendStory: id новой сторис не найден")
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

/// bot_api_id из InputPeer (для папок диалогов): та же схема, что peer_to_bot_id.
fn input_peer_to_bot_id(peer: &grammers_client::tl::enums::InputPeer) -> Option<i64> {
    use grammers_client::tl as tl;
    match peer {
        tl::enums::InputPeer::Empty | tl::enums::InputPeer::PeerSelf => None,
        tl::enums::InputPeer::User(u) => Some(u.user_id),
        tl::enums::InputPeer::Chat(c) => Some(-c.chat_id),
        tl::enums::InputPeer::Channel(c) => Some(-(1_000_000_000_000 + c.channel_id)),
        tl::enums::InputPeer::UserFromMessage(u) => Some(u.user_id),
        tl::enums::InputPeer::ChannelFromMessage(c) => {
            Some(-(1_000_000_000_000 + c.channel_id))
        }
    }
}

fn text_of(t: &grammers_client::tl::enums::TextWithEntities) -> String {
    match t {
        grammers_client::tl::enums::TextWithEntities::Entities(t) => t.text.clone(),
    }
}

/// grammers Message → MessageInfo (единая точка: history, pinned).
fn message_to_info(m: &grammers_client::message::Message, chat_id: i64) -> MessageInfo {
    use grammers_client::media::Media as M;
    let mut kind: Option<String> = None;
    let mut name: Option<String> = None;
    let mut size: Option<i64> = None;
    let mut poll: Option<super::PollInfo> = None;
    let mut geo: Option<(f64, f64)> = None;
    let mut contact: Option<(String, String)> = None;
    match m.media() {
        None => {}
        Some(media) => match &media {
            M::Photo(p) => {
                kind = Some("photo".into());
                name = Some(format!("photo_{}.jpg", m.id()));
                size = p.size().map(|s| s as i64);
            }
            M::Document(d) => {
                let fname = d.name().map(|s| s.to_string());
                let mime = d.mime_type().unwrap_or("");
                let k = if mime.starts_with("video") {
                    "video"
                } else if mime.starts_with("audio") {
                    "audio"
                } else {
                    "document"
                };
                kind = Some(k.into());
                name = fname.or_else(|| Some(format!("doc_{}", m.id())));
                size = d.size().map(|s| s as i64);
            }
            M::Sticker(_) => {
                kind = Some("sticker".into());
                name = Some(format!("sticker_{}.webp", m.id()));
                size = media.size().map(|s| s as i64);
            }
            M::Poll(p) => {
                // Опрос — display-only (скачивать нечего): has_media=false.
                poll = Some(poll_info(p));
            }
            M::Geo(g) => {
                kind = Some("geo".into());
                geo = Some((g.latitue(), g.longitude()));
            }
            M::Contact(c) => {
                kind = Some("contact".into());
                let full = format!("{} {}", c.first_name(), c.last_name())
                    .trim()
                    .to_string();
                contact = Some((full, c.phone_number().to_string()));
            }
            _ => {
                kind = Some("media".into());
                name = Some(format!("media_{}", m.id()));
                size = media.size().map(|s| s as i64);
            }
        },
    }
    MessageInfo {
        id: m.id(),
        chat_id,
        text: m.text().to_string(),
        date: m.date().as_second(),
        has_media: kind.is_some(),
        media_kind: kind,
        file_name: name,
        file_size: size,
        outgoing: m.outgoing(),
        sender: m.sender().and_then(|p| p.name().map(|s| s.to_string())),
        edited: m.edit_date().is_some(),
        reactions: m.reaction_count(),
        views: m.view_count(),
        forwards: m.forward_count(),
        poll,
        geo,
        contact,
    }
}

/// grammers Poll → PollInfo (варианты + голоса + выбранные).
fn poll_info(p: &grammers_client::media::Poll) -> super::PollInfo {
    use std::collections::HashMap;
    // Карта option-bytes → (votes, chosen).
    let mut stat: HashMap<Vec<u8>, (Option<i32>, bool)> = HashMap::new();
    if let Some(voters) = p.iter_voters_summary() {
        for v in voters {
            stat.insert(v.option.clone(), (v.voters, v.chosen));
        }
    }
    let answers = p
        .iter_answers()
        .map(|a| {
            let (text, option) = match a {
                grammers_client::tl::enums::PollAnswer::Answer(t) => {
                    (text_of(&t.text), t.option.clone())
                }
                _ => ("?".to_string(), vec![]),
            };
            let (votes, chosen) = stat.get(&option).copied().unwrap_or((None, false));
            super::PollAnswerInfo { text, option, votes, chosen }
        })
        .collect();
    super::PollInfo {
        question: text_of(&p.question()),
        closed: p.closed(),
        quiz: p.is_quiz(),
        total_voters: p.total_voters(),
        answers,
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
