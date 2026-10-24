//! Реальный MTProto-бэкенд на `grammers`.
//!
//! Ключевое для ТЗ:
//! - Сторис читаются через `stories.getAllStories`.
//! - Скачивание из групп с `noforwards` идёт напрямую через
//!   `client.download_media(&media, path)` — сервер отдаёт байты, потому что
//!   флаг `noforwards` лишь прячет кнопки в официальных клиентах.

use super::{
    AccountInfo, AdminRights, BlockedInfo, ChannelStats, ChatInfo, ContactInfo, DialogFilterInfo,
    MessageInfo, ParticipantInfo, PollInfo, ProfileInfo, SessionInfo, StoryAlbumInfo, StoryInfo,
    StoryViewer, TelegramService, TopicInfo, WebPreview, WebPreviewPhoto,
};
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
    /// Подключение без прокси (удобная обёртка над [`Self::connect_with_proxy`]).
    #[allow(dead_code)]
    pub async fn connect(session_path: &Path, api_id: i32, api_hash: &str) -> Result<Self> {
        Self::connect_with_proxy(session_path, api_id, api_hash, None).await
    }

    /// Подключение с опциональным SOCKS5-прокси (только socks5://…; MTProto-прокси
    /// grammers не поддерживает — задокументировано в README).
    pub async fn connect_with_proxy(
        session_path: &Path,
        api_id: i32,
        api_hash: &str,
        proxy_url: Option<String>,
    ) -> Result<Self> {
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let session =
            Arc::new(SqliteSession::open(session_path.to_string_lossy().to_string()).await?);
        let SenderPool { runner, handle, .. } = match proxy_url.filter(|u| !u.trim().is_empty()) {
            Some(url) => {
                use grammers_mtsender::ConnectionParams;
                SenderPool::with_configuration(
                    Arc::clone(&session),
                    api_id,
                    ConnectionParams {
                        proxy_url: Some(url),
                        ..Default::default()
                    },
                )
            }
            None => SenderPool::new(Arc::clone(&session), api_id),
        };
        let client = Client::new(handle);
        tokio::spawn(runner.run());
        Ok(Self {
            client,
            _session: session,
            api_id,
            api_hash: api_hash.to_string(),
            parallel_workers: std::sync::Mutex::new(crate::media::mtproto::DEFAULT_WORKERS),
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
        Ok(self
            .client
            .request_login_code(phone, &self.api_hash)
            .await?)
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
        use grammers_client::tl;
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
                out.push((
                    peer_to_bot_id(&inner.peer),
                    inner.unread_count,
                    muted,
                    archived,
                ));
            }
        }
        out
    }
    /// Залить файл и получить готовое InputMedia (фото/документ) через uploadMedia.
    /// Список диалогов (чаты/каналы/пользователи) через iter_dialogs.
    async fn dialogs(&self) -> Result<Vec<super::ChatInfo>> {
        use grammers_client::message::Message as GrMsg;
        let mut out = Vec::new();
        let mut refs: Vec<(i64, grammers_session::types::PeerRef)> = Vec::new();
        let mut iter = self.client.iter_dialogs();
        while let Some(d) = iter.next().await? {
            let bot_id = d.peer_id().bot_api_dialog_id().unwrap_or(0);
            if bot_id == 0 {
                continue;
            }
            if let Some(pref) = self.resolve_ref(bot_id).await? {
                refs.push((bot_id, pref));
            }
            let (title, username) = match d.peer() {
                grammers_client::peer::Peer::User(u) => {
                    let n = u.full_name();
                    (
                        if n.trim().is_empty() {
                            "Без имени".to_string()
                        } else {
                            n
                        },
                        u.username().map(|s| s.to_string()),
                    )
                }
                grammers_client::peer::Peer::Group(g) => (
                    g.title().unwrap_or("Группа").to_string(),
                    g.username().map(|s| s.to_string()),
                ),
                grammers_client::peer::Peer::Channel(_) => (
                    "Канал".to_string(),
                    d.peer().username().map(|s| s.to_string()),
                ),
                grammers_client::peer::Peer::Community(_) => (
                    "Сообщество".to_string(),
                    d.peer().username().map(|s| s.to_string()),
                ),
            };
            out.push(super::ChatInfo {
                id: bot_id,
                title,
                username,
                protected: Self::peer_protected(d.peer()),
                unread: 0,
                muted: false,
                archived: false,
            });
            if out.len() >= 300 {
                break;
            }
        }
        // Второй проход: статусы (непрочитанные/мьют/архив) батчем.
        for (bot_id, unread, muted, archived) in self.peer_unread_map(&refs).await {
            if let Some(c) = out.iter_mut().find(|c| c.id == bot_id) {
                c.unread = unread;
                c.muted = muted;
                c.archived = archived;
            }
        }
        // Пиннед-чаты первыми, затем по новизне (порядок iter_dialogs уже верный).
        out.sort_by_key(|c| std::cmp::Reverse(c.unread));
        Ok(out)
    }

    async fn history(&self, chat_id: i64, limit: usize) -> Result<Vec<super::MessageInfo>> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut out = Vec::new();
        let mut iter = self.client.iter_messages(pref);
        iter = iter.limit(limit.min(200));
        while let Some(m) = iter.next().await? {
            out.push(message_to_info(&m, chat_id));
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Сторис всех диалогов через stories.getAllStories (батч-запрос).
    async fn stories(&self) -> Result<Vec<super::StoryInfo>> {
        use grammers_client::tl;
        let me = self.client.get_me().await?;
        let me_id = me.id().bot_api_dialog_id().unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::stories::GetAllStories {
                state: None,
                next: false,
                hidden: false,
            })
            .await
            .context("getAllStories failed")?;
        let (mut peer_stories, users, chats) = match res {
            tl::enums::stories::AllStories::Stories(s) => (s.peer_stories, s.users, s.chats),
            _ => return Ok(vec![]),
        };
        // Имена авторов: пользователи и чаты из того же ответа.
        let mut names: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        for u in &users {
            if let tl::enums::User::User(uu) = u {
                let nm = format!(
                    "{} {}",
                    uu.first_name.clone().unwrap_or_default(),
                    uu.last_name.clone().unwrap_or_default()
                )
                .trim()
                .to_string();
                let nm = if nm.is_empty() {
                    uu.username
                        .clone()
                        .unwrap_or_else(|| format!("id{}", uu.id))
                } else {
                    nm
                };
                names.insert(uu.id, nm);
            }
        }
        for c in &chats {
            let (id, title) = match c {
                tl::enums::Chat::Channel(cc) => (cc.id, cc.title.clone()),
                tl::enums::Chat::Chat(cc) => (-cc.id, cc.title.clone()),
                _ => continue,
            };
            names.insert(id, title);
        }
        let mut out = Vec::new();
        for psp in peer_stories.drain(..) {
            let tl::enums::PeerStories::Stories(ps) = psp;
            let bot_id = peer_to_bot_id(&ps.peer);
            let title = names
                .get(&bot_id)
                .cloned()
                .unwrap_or_else(|| "Сторис".to_string());
            for st in ps.stories {
                if let tl::enums::StoryItem::Item(item) = st {
                    out.push(super::StoryInfo {
                        peer_id: bot_id,
                        peer_title: title.clone(),
                        story_id: item.id,
                        caption: crate::utils::truncate(
                            &item.caption.clone().unwrap_or_default(),
                            300,
                        ),
                        has_url: false,
                        date: item.date as i64,
                        preview_path: None,
                    });
                }
            }
        }
        let _ = me_id;
        Ok(out)
    }

    async fn me(&self) -> Result<super::AccountInfo> {
        let me = self.client.get_me().await?;
        let id = me.id().bot_api_dialog_id().unwrap_or(0);
        Ok(super::AccountInfo {
            id,
            name: me.full_name(),
            phone: me.phone().map(|s| s.to_string()),
            username: me.username().map(|s| s.to_string()),
        })
    }

    async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i32>,
        silent: bool,
    ) -> Result<i32> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let msg = self
            .client
            .send_message(
                pref,
                InputMessage::new()
                    .text(text)
                    .reply_to(reply_to)
                    .silent(silent),
            )
            .await?;
        Ok(msg.id())
    }

    async fn edit_text(&self, chat_id: i64, message_id: i32, text: &str) -> Result<()> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .edit_message(pref, message_id, InputMessage::new().text(text))
            .await?;
        Ok(())
    }

    async fn delete_messages(&self, chat_id: i64, message_ids: &[i32]) -> Result<()> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client.delete_messages(pref, message_ids).await?;
        Ok(())
    }

    async fn react(&self, chat_id: i64, message_id: i32, emoji: &str) -> Result<()> {
        use grammers_client::message::InputReactions;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let reactions = if emoji.trim().is_empty() {
            InputReactions::remove()
        } else {
            InputReactions::emoticon(emoji)
        };
        self.client
            .send_reactions(pref, message_id, reactions)
            .await?;
        Ok(())
    }

    async fn forward(&self, from_chat: i64, message_ids: &[i32], to_chat: i64) -> Result<usize> {
        let from = self
            .resolve_ref(from_chat)
            .await?
            .context("peer not found")?;
        let to = self.resolve_ref(to_chat).await?.context("peer not found")?;
        let sent = self.client.forward_messages(to, message_ids, from).await?;
        Ok(sent.iter().filter(|m| m.is_some()).count())
    }

    async fn save_draft(&self, chat_id: i64, text: &str, reply_to: Option<i32>) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let reply = reply_to.map(|id| {
            tl::enums::InputReplyTo::Message(tl::types::InputReplyToMessage {
                reply_to_msg_id: id,
                top_msg_id: None,
                reply_to_peer_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                monoforum_peer_id: None,
                todo_item_id: None,
                poll_option: None,
            })
        });
        self.client
            .invoke(&tl::functions::messages::SaveDraft {
                no_webpage: true,
                invert_media: false,
                reply_to: reply,
                peer: (&pref).into(),
                message: text.to_string(),
                entities: None,
                media: None,
                effect: None,
                suggested_post: None,
                rich_message: None,
            })
            .await?;
        Ok(())
    }

    async fn mark_read(&self, chat_id: i64) -> Result<()> {
        if let Some(pref) = self.resolve_ref(chat_id).await? {
            let _ = self.client.mark_as_read(pref).await;
        }
        Ok(())
    }

    async fn pin_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::UpdatePinnedMessage {
                silent: false,
                unpin: false,
                pm_oneside: false,
                peer: (&pref).into(),
                id: message_id,
            })
            .await?;
        Ok(())
    }

    async fn unpin_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::UpdatePinnedMessage {
                silent: true,
                unpin: true,
                pm_oneside: false,
                peer: (&pref).into(),
                id: message_id,
            })
            .await?;
        Ok(())
    }

    async fn pinned_message(&self, chat_id: i64) -> Result<Option<super::MessageInfo>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await.context("peer not found")?;
        let pref = match pref {
            Some(p) => p,
            None => return Ok(None),
        };
        let mut items = self
            .client
            .search_messages(pref)
            .filter(tl::enums::MessagesFilter::InputMessagesFilterPinned)
            .limit(1);
        if let Some(m) = items.next().await? {
            return Ok(Some(message_to_info(&m, chat_id)));
        }
        Ok(None)
    }

    async fn vote(&self, chat_id: i64, message_id: i32, option: Vec<u8>) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::SendVote {
                peer: (&pref).into(),
                msg_id: message_id,
                options: vec![option],
            })
            .await?;
        Ok(())
    }

    async fn send_file(
        &self,
        chat_id: i64,
        path: &std::path::Path,
        caption: &str,
        silent: bool,
        schedule_at: Option<std::time::SystemTime>,
    ) -> Result<i32> {
        use grammers_client::message::InputMessage;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let uploaded = self.client.upload_file(path).await?;
        let is_photo = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
            .unwrap_or(false);
        let mut msg = if is_photo {
            InputMessage::new().text(caption).photo(uploaded)
        } else {
            InputMessage::new().text(caption).document(uploaded)
        };
        msg = msg.silent(silent);
        if let Some(at) = schedule_at {
            msg = msg.schedule_date(Some(at));
        }
        let sent = self.client.send_message(pref, msg).await?;
        Ok(sent.id())
    }

    async fn send_scheduled(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i32>,
        at: std::time::SystemTime,
        silent: bool,
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
                    .silent(silent)
                    .schedule_date(Some(at)),
            )
            .await?;
        Ok(sent.id())
    }

    async fn sessions(&self) -> Result<Vec<super::SessionInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::account::GetAuthorizations {})
            .await?;
        let list = match res {
            tl::enums::account::Authorizations::Authorizations(a) => a.authorizations,
        };
        Ok(list
            .into_iter()
            .filter_map(|s| match s {
                tl::enums::Authorization::Authorization(a) => Some(super::SessionInfo {
                    current: a.current,
                    app_name: a.app_name.clone(),
                    device: format!("{} {}", a.device_model, a.system_version),
                    platform: a.platform.clone(),
                    country: a.country.clone(),
                    ip: a.ip.clone(),
                    last_active: a.date_active as i64,
                    hash: a.hash,
                }),
                _ => None,
            })
            .collect())
    }

    async fn terminate_others(&self) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::auth::ResetAuthorizations {})
            .await?;
        Ok(())
    }

    async fn mute(&self, chat_id: i64, muted: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mute_until = if muted { Some(i32::MAX) } else { Some(0) };
        self.client
            .invoke(&tl::functions::account::UpdateNotifySettings {
                peer: tl::enums::InputNotifyPeer::Peer(tl::types::InputNotifyPeer {
                    peer: (&pref).into(),
                }),
                settings: tl::enums::InputPeerNotifySettings::Settings(
                    tl::types::InputPeerNotifySettings {
                        show_previews: None,
                        silent: None,
                        mute_until,
                        sound: None,
                        stories_muted: None,
                        stories_hide_sender: None,
                        stories_sound: None,
                    },
                ),
            })
            .await?;
        Ok(())
    }

    async fn archive(&self, chat_id: i64, archived: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::folders::EditPeerFolders {
                folder_peers: vec![tl::enums::InputFolderPeer::Peer(
                    tl::types::InputFolderPeer {
                        peer: (&pref).into(),
                        folder_id: if archived { 1 } else { 0 },
                    },
                )],
            })
            .await?;
        Ok(())
    }

    async fn contacts(&self) -> Result<Vec<super::ContactInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::contacts::GetContacts { hash: 0 })
            .await?;
        let users = match res {
            tl::enums::contacts::Contacts::Contacts(c) => c.users,
            _ => vec![],
        };
        let mut out = Vec::new();
        for u in users {
            if let tl::enums::User::User(u) = u {
                let name = match (&u.first_name, &u.last_name) {
                    (Some(f), Some(l)) => format!("{f} {l}"),
                    (Some(f), None) => f.clone(),
                    (None, Some(l)) => l.clone(),
                    _ => u.username.clone().unwrap_or_else(|| format!("id{}", u.id)),
                };
                out.push(super::ContactInfo {
                    name,
                    phone: u.phone.unwrap_or_default(),
                    username: u.username.clone(),
                    user_id: u.id,
                    access_hash: u.access_hash.unwrap_or(0),
                });
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    async fn dialog_filters(&self) -> Result<Vec<super::DialogFilterInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::messages::GetDialogFilters {})
            .await?;
        let filters = match res {
            tl::enums::messages::DialogFilters::Filters(f) => f.filters,
        };
        let mut out = Vec::new();
        for f in filters {
            if let tl::enums::DialogFilter::Filter(df) = f {
                let title = match &df.title {
                    tl::enums::TextWithEntities::Entities(t) => t.text.clone(),
                    _ => String::new(),
                };
                if title.is_empty() {
                    continue;
                }
                let mut peer_ids = Vec::new();
                for p in df.pinned_peers.iter().chain(df.include_peers.iter()) {
                    if let Some(id) = input_peer_to_bot_id(p) {
                        peer_ids.push(id);
                    }
                }
                out.push(super::DialogFilterInfo {
                    id: df.id,
                    title,
                    peer_ids,
                });
            }
        }
        Ok(out)
    }

    async fn update_profile(&self, first_name: &str, last_name: &str, about: &str) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::account::UpdateProfile {
                first_name: if first_name.is_empty() {
                    None
                } else {
                    Some(first_name.to_string())
                },
                last_name: if last_name.is_empty() {
                    None
                } else {
                    Some(last_name.to_string())
                },
                about: if about.is_empty() {
                    None
                } else {
                    Some(about.to_string())
                },
            })
            .await?;
        Ok(())
    }

    async fn get_profile(&self) -> Result<super::ProfileInfo> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::users::GetFullUser {
                id: tl::enums::InputUser::UserSelf,
            })
            .await?;
        let about = match res {
            tl::enums::users::UserFull::Full(f) => match &f.full_user {
                tl::enums::UserFull::Full(inner) => inner.about.clone(),
                _ => None,
            },
            _ => None,
        };
        let me = self.client.get_me().await?;
        Ok(super::ProfileInfo {
            first_name: me.first_name().unwrap_or("").to_string(),
            last_name: me.last_name().unwrap_or("").to_string(),
            about: about.unwrap_or_default(),
            username: me.username().map(|s| s.to_string()),
        })
    }

    async fn download_message_media(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        std::fs::create_dir_all(dest_dir).ok();
        let mut msgs = self
            .client
            .get_messages_by_id(pref.clone(), &[message_id])
            .await?;
        let msg = msgs
            .drain(..)
            .flatten()
            .next()
            .context("сообщение не найдено — возможно, удалено")?;
        let media = msg.media().context("у сообщения нет медиа")?;
        // Имя файла: как в скачанных (консистентно с UI/БД).
        let fname = media_file_name(&media, message_id);
        let dest = dest_dir.join(fname);
        self.client.download_media(&media, &dest).await?;
        Ok(dest)
    }

    async fn input_media_for_file(
        &self,
        path: &Path,
        peer: grammers_client::tl::enums::InputPeer,
    ) -> Result<grammers_client::tl::enums::InputMedia> {
        use grammers_client::tl;
        let uploaded = self
            .client
            .upload_file(path)
            .await
            .with_context(|| format!("upload failed: {}", path.display()))?;
        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file.bin".to_string());
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
                    tl::enums::DocumentAttribute::Filename(tl::types::DocumentAttributeFilename {
                        file_name: fname,
                    }),
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
        let uploaded_media = self
            .client
            .invoke(&tl::functions::messages::UploadMedia {
                business_connection_id: None,
                peer,
                media: upload_media,
            })
            .await
            .context("uploadMedia failed")?;
        match uploaded_media {
            tl::enums::MessageMedia::Photo(p) => {
                let photo = p.photo.context("uploadMedia: no photo")?;
                let (id, access_hash, file_reference) = match photo {
                    tl::enums::Photo::Photo(p) => (p.id, p.access_hash, p.file_reference),
                    _ => anyhow::bail!("uploadMedia: unsupported photo"),
                };
                Ok(tl::enums::InputMedia::Photo(tl::types::InputMediaPhoto {
                    spoiler: false,
                    live_photo: false,
                    id: tl::enums::InputPhoto::Photo(tl::types::InputPhoto {
                        id,
                        access_hash,
                        file_reference,
                    }),
                    ttl_seconds: None,
                    video: None,
                }))
            }
            tl::enums::MessageMedia::Document(d) => {
                let doc = d.document.context("uploadMedia: no document")?;
                let (id, access_hash, file_reference) = match doc {
                    tl::enums::Document::Document(d) => (d.id, d.access_hash, d.file_reference),
                    _ => anyhow::bail!("uploadMedia: unsupported document"),
                };
                Ok(tl::enums::InputMedia::Document(
                    tl::types::InputMediaDocument {
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
                    },
                ))
            }
            _ => anyhow::bail!("uploadMedia: неожиданный тип медиа"),
        }
    }

    async fn publish_story(
        &self,
        path: &Path,
        caption: &str,
        music: Option<std::path::PathBuf>,
    ) -> Result<i32> {
        use grammers_client::tl;
        let media = self
            .input_media_for_file(path, tl::enums::InputPeer::PeerSelf)
            .await?;
        // Опциональная музыка: аудиофайл → InputDocument.
        let music_doc = if let Some(mp3) = music.as_deref() {
            let up = self
                .client
                .upload_file(mp3)
                .await
                .context("music upload failed")?;
            let fname = mp3
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "music.mp3".to_string());
            let res = self
                .client
                .invoke(&tl::functions::messages::UploadMedia {
                    business_connection_id: None,
                    peer: tl::enums::InputPeer::PeerSelf,
                    media: tl::enums::InputMedia::UploadedDocument(
                        tl::types::InputMediaUploadedDocument {
                            nosound_video: false,
                            force_file: false,
                            spoiler: false,
                            file: up.raw,
                            thumb: None,
                            mime_type: "audio/mpeg".to_string(),
                            attributes: vec![
                                tl::enums::DocumentAttribute::Filename(
                                    tl::types::DocumentAttributeFilename { file_name: fname },
                                ),
                                tl::enums::DocumentAttribute::Audio(
                                    tl::types::DocumentAttributeAudio {
                                        voice: false,
                                        duration: 0,
                                        title: None,
                                        performer: None,
                                        waveform: None,
                                    },
                                ),
                            ],
                            stickers: None,
                            video_cover: None,
                            video_timestamp: None,
                            ttl_seconds: None,
                        },
                    ),
                })
                .await
                .context("music uploadMedia failed")?;
            match res {
                tl::enums::MessageMedia::Document(d) => {
                    let doc = d.document.context("music: no document")?;
                    let (id, access_hash, file_reference) = match doc {
                        tl::enums::Document::Document(d) => (d.id, d.access_hash, d.file_reference),
                        _ => anyhow::bail!("music: unsupported document"),
                    };
                    Some(tl::enums::InputDocument::Document(
                        tl::types::InputDocument {
                            id,
                            access_hash,
                            file_reference,
                        },
                    ))
                }
                _ => anyhow::bail!("music: неожиданный тип медиа"),
            }
        } else {
            None
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
                caption: if caption.is_empty() {
                    None
                } else {
                    Some(caption.to_string())
                },
                entities: None,
                privacy_rules: vec![tl::enums::InputPrivacyRule::InputPrivacyValueAllowContacts],
                random_id,
                period: None,
                fwd_from_id: None,
                fwd_from_story: None,
                albums: None,
                music: music_doc,
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

    async fn delete_stories(&self, story_ids: &[i32]) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::stories::DeleteStories {
                peer: tl::enums::InputPeer::PeerSelf,
                id: story_ids.to_vec(),
            })
            .await
            .context("deleteStories failed")?;
        Ok(())
    }

    async fn forum_topics(&self, chat_id: i64) -> Result<Vec<super::TopicInfo>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let res = match self
            .client
            .invoke(&tl::functions::messages::GetForumTopics {
                peer: (&pref).into(),
                q: None,
                offset_date: 0,
                offset_id: 0,
                offset_topic: 0,
                limit: 100,
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("getForumTopics failed (не форум?): {e}");
                return Ok(vec![]);
            }
        };
        let t = match res {
            tl::enums::messages::ForumTopics::Topics(t) => t,
        };
        let mut out = Vec::new();
        for f in t.topics {
            if let tl::enums::ForumTopic::Topic(topic) = f {
                out.push(super::TopicInfo {
                    id: topic.id,
                    title: topic.title,
                });
            }
        }
        Ok(out)
    }

    async fn create_topic(&self, chat_id: i64, title: &str) -> Result<i32> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::messages::CreateForumTopic {
                title_missing: false,
                peer: (&pref).into(),
                title: title.to_string(),
                icon_color: None,
                icon_emoji_id: None,
                random_id,
                send_as: None,
            })
            .await
            .context("createForumTopic failed")?;
        // Ответ — Updates: ищем id нового топика.
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => vec![],
        };
        for u in &updates {
            // Новый топик приходит служебным сообщением с reply_to header;
            // надёжнее — перечитать список и взять максимальный id.
            let _ = u;
        }
        let topics = self.forum_topics(chat_id).await?;
        topics
            .into_iter()
            .map(|t| t.id)
            .max()
            .context("createForumTopic: топик не найден после создания")
    }

    async fn topic_history(
        &self,
        chat_id: i64,
        topic_id: i32,
        limit: usize,
    ) -> Result<Vec<MessageInfo>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // messages.search с top_msg_id — единственный способ читать топик
        // (у HistoryIter топ-фильтра нет, пустой q + фильтр Empty = весь топик).
        let res = self
            .client
            .invoke(&tl::functions::messages::Search {
                peer: (&pref).into(),
                q: String::new(),
                from_id: None,
                saved_peer_id: None,
                saved_reaction: None,
                top_msg_id: Some(topic_id),
                filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
                min_date: 0,
                max_date: 0,
                offset_id: 0,
                add_offset: 0,
                limit: limit.min(200) as i32,
                max_id: 0,
                min_id: 0,
                hash: 0,
            })
            .await
            .context("topic search failed")?;
        let (messages, users) = match res {
            tl::enums::messages::Messages::Messages(m) => (m.messages, vec![]),
            tl::enums::messages::Messages::Slice(m) => (m.messages, vec![]),
            tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, m.users),
            _ => (vec![], vec![]),
        };
        let names = users_map(&users);
        Ok(messages
            .into_iter()
            .filter_map(|m| raw_to_info(&m, chat_id, Some(topic_id), &names))
            .collect())
    }

    async fn send_to_topic(
        &self,
        chat_id: i64,
        topic_id: i32,
        text: &str,
        silent: bool,
    ) -> Result<i32> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let (text, entities) = grammers_client::parsers::parse_markdown_message(text);
        let entities = if entities.is_empty() {
            None
        } else {
            Some(entities)
        };
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_nanos() & 0x7FFF_FFFF_FFFF_FFFF) as i64)
            .unwrap_or(1);
        // Сырой sendMessage с top_msg_id — grammers InputMessage топики не умеет.
        let res = self
            .client
            .invoke(&tl::functions::messages::SendMessage {
                no_webpage: true,
                silent,
                background: false,
                clear_draft: true,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                peer: (&pref).into(),
                reply_to: Some(tl::enums::InputReplyTo::Message(
                    tl::types::InputReplyToMessage {
                        reply_to_msg_id: topic_id,
                        top_msg_id: Some(topic_id),
                        reply_to_peer_id: None,
                        quote_text: None,
                        quote_entities: None,
                        quote_offset: None,
                        monoforum_peer_id: None,
                        todo_item_id: None,
                        poll_option: None,
                    },
                )),
                message: text,
                random_id,
                reply_markup: None,
                entities,
                schedule_date: None,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                allow_paid_floodskip: false,
                allow_paid_stars: None,
                effect: None,
                suggested_post: None,
                rich_message: None,
            })
            .await
            .context("send to topic failed")?;
        // Ответ — Updates: вытаскиваем id отправленного из NewMessage.
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => vec![],
        };
        for u in &updates {
            match u {
                tl::enums::Update::NewMessage(m) => {
                    if let tl::enums::Message::Message(mm) = &m.message {
                        return Ok(mm.id);
                    }
                }
                tl::enums::Update::NewChannelMessage(m) => {
                    if let tl::enums::Message::Message(mm) = &m.message {
                        return Ok(mm.id);
                    }
                }
                _ => {}
            }
        }
        anyhow::bail!("send to topic: id не найден в Updates")
    }

    async fn global_search(&self, query: &str, limit: usize) -> Result<Vec<(i64, MessageInfo)>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let mut iter = self
            .client
            .search_all_messages()
            .query(q)
            .limit(limit.min(100));
        let mut out = Vec::new();
        while let Some(m) = iter.next().await? {
            let pref = m
                .peer_ref()
                .await
                .map_err(|e| anyhow::anyhow!("peer_ref: {e}"))?
                .context("global search: no peer")?;
            let bot_id = pref.id.bot_api_dialog_id().unwrap_or(0);
            out.push((bot_id, message_to_info(&m, bot_id)));
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    async fn scheduled_history(&self, chat_id: i64) -> Result<Vec<MessageInfo>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let res = self
            .client
            .invoke(&tl::functions::messages::GetScheduledHistory {
                peer: (&pref).into(),
                hash: 0,
            })
            .await
            .context("getScheduledHistory failed")?;
        let (messages, users) = match res {
            tl::enums::messages::Messages::Messages(m) => (m.messages, vec![]),
            tl::enums::messages::Messages::Slice(m) => (m.messages, vec![]),
            tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, m.users),
            _ => (vec![], vec![]),
        };
        let names = users_map(&users);
        Ok(messages
            .into_iter()
            .filter_map(|m| raw_to_info(&m, chat_id, None, &names))
            .collect())
    }

    async fn cancel_scheduled(&self, chat_id: i64, message_id: i32) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::DeleteScheduledMessages {
                peer: (&pref).into(),
                id: vec![message_id],
            })
            .await
            .context("deleteScheduledMessages failed")?;
        Ok(())
    }

    async fn create_channel(&self, title: &str, about: &str, broadcast: bool) -> Result<i64> {
        use grammers_client::tl;
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("пустое название");
        }
        let res = self
            .client
            .invoke(&tl::functions::channels::CreateChannel {
                broadcast,
                megagroup: !broadcast,
                for_import: false,
                forum: false,
                title: title.to_string(),
                about: about.to_string(),
                geo_point: None,
                address: None,
                ttl_period: None,
            })
            .await
            .context("createChannel failed")?;
        // Ответ — Updates с новым чатом: берём первый Channel.
        let (updates, chats) = match res {
            tl::enums::Updates::Updates(u) => (u.updates, u.chats),
            tl::enums::Updates::Combined(u) => (u.updates, u.chats),
            _ => (vec![], vec![]),
        };
        let _ = updates;
        for c in &chats {
            if let tl::enums::Chat::Channel(ch) = c {
                return Ok(-(1_000_000_000_000 + ch.id));
            }
        }
        anyhow::bail!("createChannel: новый чат не найден в Updates")
    }

    async fn invite_link(&self, chat_id: i64) -> Result<String> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let res = self
            .client
            .invoke(&tl::functions::messages::ExportChatInvite {
                legacy_revoke_permanent: false,
                request_needed: false,
                peer: (&pref).into(),
                expire_date: None,
                usage_limit: None,
                title: None,
                subscription_pricing: None,
            })
            .await
            .context("exportChatInvite failed (нет прав?)")?;
        match res {
            tl::enums::ExportedChatInvite::ChatInviteExported(e) => Ok(e.link),
            _ => anyhow::bail!("неожиданный тип инвайта"),
        }
    }

    async fn block_user(&self, chat_id: i64, blocked: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        if blocked {
            self.client
                .invoke(&tl::functions::contacts::Block {
                    my_stories_from: false,
                    id: (&pref).into(),
                })
                .await
                .context("block failed")?;
        } else {
            self.client
                .invoke(&tl::functions::contacts::Unblock {
                    my_stories_from: false,
                    id: (&pref).into(),
                })
                .await
                .context("unblock failed")?;
        }
        Ok(())
    }

    async fn clear_history(&self, chat_id: i64, revoke: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // Каналы/супергруппы — через channels.deleteHistory, иначе messages.
        if pref.id.kind() == grammers_session::types::PeerKind::Channel {
            let channel: tl::enums::InputChannel = pref.into();
            self.client
                .invoke(&tl::functions::channels::DeleteHistory {
                    for_everyone: revoke,
                    channel,
                    max_id: 0,
                })
                .await
                .context("clear history failed (нет прав?)")?;
        } else {
            self.client
                .invoke(&tl::functions::messages::DeleteHistory {
                    just_clear: true,
                    revoke,
                    peer: pref.into(),
                    max_id: 0,
                    min_date: None,
                    max_date: None,
                })
                .await
                .context("clear history failed")?;
        }
        Ok(())
    }

    async fn leave_channel(&self, chat_id: i64) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        self.client
            .invoke(&tl::functions::channels::LeaveChannel { channel })
            .await
            .context("leave failed")?;
        Ok(())
    }

    async fn add_contact(&self, phone: &str, first_name: &str, last_name: &str) -> Result<()> {
        use grammers_client::tl;
        let phone = phone.trim().replace([' ', '-', '(', ')'], "");
        if phone.len() < 5 || first_name.trim().is_empty() {
            anyhow::bail!("нужны телефон и имя");
        }
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        self.client
            .invoke(&tl::functions::contacts::ImportContacts {
                contacts: vec![tl::enums::InputContact::InputPhoneContact(
                    tl::types::InputPhoneContact {
                        client_id: random_id,
                        phone,
                        first_name: first_name.trim().to_string(),
                        last_name: last_name.trim().to_string(),
                        note: None,
                    },
                )],
            })
            .await
            .context("importContacts failed")?;
        Ok(())
    }

    async fn download_web_photo(
        &self,
        photo: &super::WebPreviewPhoto,
        dest: &Path,
    ) -> Result<PathBuf> {
        use grammers_client::tl;
        // Собираем grammers::Photo из сохранённых id/hash/ref (sizes пустые —
        // download возьмёт thumb_size "w", dc — из dc_id).
        let raw = tl::enums::Photo::Photo(tl::types::Photo {
            has_stickers: false,
            id: photo.photo_id,
            access_hash: photo.access_hash,
            file_reference: photo.file_reference.clone(),
            date: 0,
            sizes: vec![],
            video_sizes: None,
            dc_id: photo.dc_id,
        });
        let media =
            grammers_client::media::Media::Photo(grammers_client::media::Photo::from_raw(raw));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        self.client
            .download_media(&media, dest)
            .await
            .context("web photo download failed")?;
        Ok(dest.to_path_buf())
    }

    async fn create_poll(
        &self,
        chat_id: i64,
        question: &str,
        options: &[String],
        quiz: bool,
        correct: Option<usize>,
        anonymous: bool,
        multiple_choice: bool,
    ) -> Result<i32> {
        use grammers_client::message::InputMessage;
        use grammers_client::tl;
        let question = question.trim();
        let options: Vec<String> = options
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if question.is_empty() || options.len() < 2 || options.len() > 10 {
            anyhow::bail!("вопрос + от 2 до 10 вариантов");
        }
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let rnd = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0)
        };
        let txt = |t: String| {
            tl::enums::TextWithEntities::Entities(tl::types::TextWithEntities {
                text: t,
                entities: vec![],
            })
        };
        let poll = tl::enums::Poll::Poll(tl::types::Poll {
            id: rnd(),
            closed: false,
            public_voters: !anonymous,
            multiple_choice,
            quiz,
            open_answers: false,
            revoting_disabled: false,
            shuffle_answers: false,
            hide_results_until_close: false,
            creator: quiz,
            subscribers_only: false,
            question: txt(question.to_string()),
            answers: options
                .into_iter()
                .enumerate()
                .map(|(i, text)| {
                    tl::enums::PollAnswer::Answer(tl::types::PollAnswer {
                        text: txt(text),
                        option: format!("opt{i}").into_bytes(),
                        media: None,
                        added_by: None,
                        date: None,
                    })
                })
                .collect(),
            close_period: None,
            close_date: None,
            countries_iso2: None,
            hash: rnd(),
        });
        let input = InputMessage::new()
            .clear_draft(true)
            .media(tl::enums::InputMedia::Poll(Box::new(
                tl::types::InputMediaPoll {
                    poll,
                    correct_answers: if quiz {
                        correct.map(|i| vec![i as i32])
                    } else {
                        None
                    },
                    attached_media: None,
                    solution: None,
                    solution_entities: None,
                    solution_media: None,
                },
            )));
        let sent = self
            .client
            .send_message(pref, input)
            .await
            .context("createPoll failed")?;
        Ok(sent.id())
    }

    async fn send_geo(&self, chat_id: i64, lat: f64, long: f64) -> Result<i32> {
        use grammers_client::message::InputMessage;
        use grammers_client::tl;
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&long) {
            anyhow::bail!("неверные координаты");
        }
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let input = InputMessage::new()
            .clear_draft(true)
            .media(tl::enums::InputMedia::GeoPoint(
                tl::types::InputMediaGeoPoint {
                    geo_point: tl::enums::InputGeoPoint::Point(tl::types::InputGeoPoint {
                        lat,
                        long,
                        accuracy_radius: None,
                    }),
                },
            ));
        let sent = self
            .client
            .send_message(pref, input)
            .await
            .context("sendGeo failed")?;
        Ok(sent.id())
    }

    async fn story_reply(&self, peer_id: i64, story_id: i32, text: &str) -> Result<i32> {
        use grammers_client::tl;
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("пустой ответ");
        }
        let pref = self.resolve_ref(peer_id).await?.context("peer not found")?;
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_nanos() & 0x7FFF_FFFF_FFFF_FFFF) as i64)
            .unwrap_or(1);
        // Сырой SendMessage: grammers InputMessage не умеет reply_to сторис.
        let res = self
            .client
            .invoke(&tl::functions::messages::SendMessage {
                no_webpage: true,
                silent: false,
                background: false,
                clear_draft: true,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                peer: (&pref).into(),
                reply_to: Some(tl::enums::InputReplyTo::Story(
                    tl::types::InputReplyToStory {
                        peer: (&pref).into(),
                        story_id,
                    },
                )),
                message: text.to_string(),
                random_id,
                reply_markup: None,
                entities: None,
                schedule_date: None,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                allow_paid_floodskip: false,
                allow_paid_stars: None,
                effect: None,
                suggested_post: None,
                rich_message: None,
            })
            .await
            .context("story reply failed")?;
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => vec![],
        };
        for u in &updates {
            match u {
                tl::enums::Update::NewMessage(m) => {
                    if let tl::enums::Message::Message(mm) = &m.message {
                        return Ok(mm.id);
                    }
                }
                tl::enums::Update::NewChannelMessage(m) => {
                    if let tl::enums::Message::Message(mm) = &m.message {
                        return Ok(mm.id);
                    }
                }
                _ => {}
            }
        }
        anyhow::bail!("story reply: id не найден в Updates")
    }

    async fn story_views(&self, story_id: i32) -> Result<i32> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::stories::GetStoryViewsList {
                just_contacts: true,
                reactions_first: false,
                forwards_first: false,
                peer: tl::enums::InputPeer::PeerSelf,
                q: None,
                id: story_id,
                offset: String::new(),
                limit: 1,
            })
            .await
            .context("getStoryViewsList failed")?;
        match res {
            tl::enums::stories::StoryViewsList::List(l) => Ok(l.views_count),
        }
    }

    async fn enable_stealth(&self) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::stories::ActivateStealthMode {
                past: true,
                future: true,
            })
            .await
            .context("stealth failed")?;
        Ok(())
    }

    async fn story_react(&self, peer_id: i64, story_id: i32, emoji: &str) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(peer_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::stories::SendReaction {
                add_to_recent: false,
                peer: (&pref).into(),
                story_id,
                reaction: tl::enums::Reaction::Emoji(tl::types::ReactionEmoji {
                    emoticon: emoji.to_string(),
                }),
            })
            .await
            .context("story react failed")?;
        Ok(())
    }

    async fn toggle_story_pin(&self, story_id: i32, pinned: bool) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::stories::TogglePinned {
                peer: tl::enums::InputPeer::PeerSelf,
                id: vec![story_id],
                pinned,
            })
            .await
            .context("toggle story pin failed")?;
        Ok(())
    }

    async fn story_viewers(
        &self,
        story_id: i32,
        limit: usize,
    ) -> Result<(i32, Vec<super::StoryViewer>)> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::stories::GetStoryViewsList {
                just_contacts: false,
                reactions_first: false,
                forwards_first: false,
                peer: tl::enums::InputPeer::PeerSelf,
                q: None,
                id: story_id,
                offset: String::new(),
                limit: limit.min(100) as i32,
            })
            .await
            .context("getStoryViewsList failed")?;
        let list = match res {
            tl::enums::stories::StoryViewsList::List(l) => l,
        };
        let mut names: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        for u in &list.users {
            if let tl::enums::User::User(u) = u {
                let name = match (&u.first_name, &u.last_name) {
                    (Some(f), Some(l)) => format!("{f} {l}"),
                    (Some(f), None) => f.clone(),
                    (None, Some(l)) => l.clone(),
                    (None, Some(_)) => unreachable!(),
                    (None, None) => u.username.clone().unwrap_or_else(|| format!("id{}", u.id)),
                };
                names.insert(u.id, name);
            }
        }
        let mut viewers = Vec::new();
        for v in &list.views {
            let uid = match v {
                tl::enums::StoryView::View(v) => Some(v.user_id),
                _ => None,
            };
            if let Some(uid) = uid {
                if let Some(name) = names.get(&uid).cloned() {
                    viewers.push(super::StoryViewer { name });
                }
            }
            if viewers.len() >= limit {
                break;
            }
        }
        Ok((list.views_count, viewers))
    }

    async fn search_media(
        &self,
        chat_id: i64,
        kind: &str,
        limit: usize,
    ) -> Result<Vec<MessageInfo>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let filter = match kind {
            "photo" => tl::enums::MessagesFilter::InputMessagesFilterPhotos,
            "video" => tl::enums::MessagesFilter::InputMessagesFilterVideo,
            _ => tl::enums::MessagesFilter::InputMessagesFilterUrl,
        };
        let res = self
            .client
            .invoke(&tl::functions::messages::Search {
                peer: (&pref).into(),
                q: String::new(),
                from_id: None,
                saved_peer_id: None,
                saved_reaction: None,
                top_msg_id: None,
                filter,
                min_date: 0,
                max_date: 0,
                offset_id: 0,
                add_offset: 0,
                limit: limit.min(100) as i32,
                max_id: 0,
                min_id: 0,
                hash: 0,
            })
            .await
            .context("media search failed")?;
        let (messages, users) = match res {
            tl::enums::messages::Messages::Messages(m) => (m.messages, vec![]),
            tl::enums::messages::Messages::Slice(m) => (m.messages, vec![]),
            tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, m.users),
            _ => (vec![], vec![]),
        };
        let names = users_map(&users);
        Ok(messages
            .into_iter()
            .filter_map(|m| raw_to_info(&m, chat_id, None, &names))
            .collect())
    }

    async fn update_username(&self, username: &str) -> Result<()> {
        use grammers_client::tl;
        let username = username.trim().trim_start_matches('@').to_string();
        if username.is_empty() {
            anyhow::bail!("пустой username");
        }
        self.client
            .invoke(&tl::functions::account::UpdateUsername { username })
            .await
            .context("updateUsername failed (занят?)")?;
        Ok(())
    }

    async fn upload_avatar(&self, path: &Path) -> Result<()> {
        use grammers_client::tl;
        let uploaded = self
            .client
            .upload_file(path)
            .await
            .context("upload failed")?;
        self.client
            .invoke(&tl::functions::photos::UploadProfilePhoto {
                fallback: false,
                bot: None,
                file: Some(uploaded.raw),
                video: None,
                video_start_ts: None,
                video_emoji_markup: None,
            })
            .await
            .context("uploadProfilePhoto failed")?;
        Ok(())
    }

    async fn delete_contact(&self, user_id: i64, access_hash: i64) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::contacts::DeleteContacts {
                id: vec![tl::enums::InputUser::User(tl::types::InputUser {
                    user_id,
                    access_hash,
                })],
            })
            .await
            .context("deleteContacts failed")?;
        Ok(())
    }

    async fn blocked_list(&self) -> Result<Vec<super::BlockedInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::contacts::GetBlocked {
                my_stories_from: false,
                offset: 0,
                limit: 100,
            })
            .await
            .context("getBlocked failed")?;
        let (blocked, users) = match res {
            tl::enums::contacts::Blocked::Blocked(b) => (b.blocked, b.users),
            tl::enums::contacts::Blocked::Slice(s) => (s.blocked, s.users),
        };
        // Имена из users ответа; чаты показываем по id.
        let names = users_map(&users);
        let mut out = Vec::new();
        for b in blocked {
            let peer_id = match b {
                tl::enums::PeerBlocked::Blocked(p) => p.peer_id,
                _ => continue,
            };
            let id = peer_to_bot_id(&peer_id);
            let name = names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("id {id}"));
            out.push(super::BlockedInfo { id, name });
        }
        Ok(out)
    }

    async fn share_contact(
        &self,
        chat_id: i64,
        phone: &str,
        first_name: &str,
        last_name: &str,
    ) -> Result<i32> {
        use grammers_client::message::InputMessage;
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let input = InputMessage::new()
            .clear_draft(true)
            .media(tl::enums::InputMedia::Contact(
                tl::types::InputMediaContact {
                    phone_number: phone.to_string(),
                    first_name: first_name.to_string(),
                    last_name: last_name.to_string(),
                    vcard: String::new(),
                },
            ));
        let sent = self
            .client
            .send_message(pref, input)
            .await
            .context("shareContact failed")?;
        Ok(sent.id())
    }

    async fn stop_poll(&self, chat_id: i64, message_id: i32) -> Result<()> {
        use grammers_client::message::InputMessage;
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // Закрытие = правка тем же опросом с closed=true.
        let msgs = self.client.get_messages_by_id(pref, &[message_id]).await?;
        let mut closed = msgs
            .into_iter()
            .next()
            .flatten()
            .and_then(|m| match m.media() {
                Some(grammers_client::media::Media::Poll(p)) => Some(p.raw.clone()),
                _ => None,
            })
            .context("not a poll message")?;
        closed.closed = true;
        let input = InputMessage::new()
            .text("")
            .media(tl::enums::InputMedia::Poll(Box::new(
                tl::types::InputMediaPoll {
                    poll: tl::enums::Poll::Poll(closed),
                    correct_answers: None,
                    attached_media: None,
                    solution: None,
                    solution_entities: None,
                    solution_media: None,
                },
            )));
        self.client
            .edit_message(pref, message_id, input)
            .await
            .context("stopPoll failed")?;
        Ok(())
    }

    async fn pinned_list(&self, chat_id: i64, limit: usize) -> Result<Vec<MessageInfo>> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut out = Vec::new();
        // Закрепы — через поиск с pinned-фильтром (порядок сервера = порядок закрепа).
        let mut iter = self
            .client
            .search_messages(pref)
            .filter(grammers_client::tl::enums::MessagesFilter::InputMessagesFilterPinned)
            .limit(limit.min(20));
        while let Some(m) = iter.next().await? {
            out.push(message_to_info(&m, chat_id));
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    async fn revoke_invite(&self, chat_id: i64, link: &str) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::DeleteExportedChatInvite {
                peer: (&pref).into(),
                link: link.to_string(),
            })
            .await
            .context("revoke invite failed")?;
        Ok(())
    }

    async fn translate_message(
        &self,
        chat_id: i64,
        message_id: i32,
        to_lang: &str,
    ) -> Result<String> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let res = self
            .client
            .invoke(&tl::functions::messages::TranslateText {
                peer: Some((&pref).into()),
                id: Some(vec![message_id]),
                text: None,
                to_lang: to_lang.to_string(),
                tone: None,
            })
            .await
            .context("translate failed (нужен Premium?)")?;
        let items = match res {
            tl::enums::messages::TranslatedText::TranslateResult(r) => r.result,
            _ => vec![],
        };
        let mut out = String::new();
        for t in items {
            if let tl::enums::TextWithEntities::Entities(e) = t {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&e.text);
            }
        }
        if out.trim().is_empty() {
            anyhow::bail!("пустой перевод");
        }
        Ok(out)
    }

    async fn download_story_media(
        &self,
        peer_id: i64,
        story_id: i32,
        dest_dir: &Path,
    ) -> Result<PathBuf> {
        use grammers_client::tl;
        std::fs::create_dir_all(dest_dir).ok();
        let pref = self.resolve_ref(peer_id).await?.context("peer not found")?;
        let stories = self
            .client
            .invoke(&tl::functions::stories::GetStoriesById {
                peer: (&pref).into(),
                id: vec![story_id],
            })
            .await
            .context("getStoriesByID failed")?;
        let media = extract_story_media(&stories).context("в сторис нет медиа")?;
        let dest = dest_dir.join(format!("repost_{peer_id}_{story_id}"));
        let is_photo = matches!(media, grammers_client::tl::enums::MessageMedia::Photo(_));
        let dest = if is_photo {
            dest.with_extension("jpg")
        } else {
            dest.with_extension("mp4")
        };
        let gm = grammers_client::media::Media::from_raw(media)
            .context("неподдерживаемый тип медиа сторис")?;
        self.client.download_media(&gm, &dest).await?;
        Ok(dest)
    }

    async fn forward_story(&self, peer_id: i64, story_id: i32, caption: &str) -> Result<i32> {
        use grammers_client::tl;
        // Шаг 1: скачать медиа чужой сторис в кэш.
        let dir = std::env::temp_dir().join("telecomfort").join("repost");
        let path = self.download_story_media(peer_id, story_id, &dir).await?;
        // Шаг 2: залить как обычный медиа-файл.
        let me_id = self
            .client
            .get_me()
            .await?
            .id()
            .bot_api_dialog_id()
            .unwrap_or(0);
        let me_pref = self
            .resolve_ref(me_id)
            .await?
            .context("me peer not found")?;
        let media = self.input_media_for_file(&path, (&me_pref).into()).await?;
        // Шаг 3: sendStory себе с fwd_from_* (репост с атрибуцией автора).
        let from_pref = self
            .resolve_ref(peer_id)
            .await?
            .context("author not found")?;
        let input_peer: tl::enums::InputPeer = (&from_pref).into();
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
                caption: if caption.is_empty() {
                    None
                } else {
                    Some(caption.to_string())
                },
                entities: None,
                privacy_rules: vec![tl::enums::InputPrivacyRule::InputPrivacyValueAllowContacts],
                random_id,
                period: None,
                fwd_from_id: Some(input_peer),
                fwd_from_story: Some(story_id),
                albums: None,
                music: None,
            })
            .await
            .context("repost failed")?;
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => vec![],
        };
        for u in updates {
            if let tl::enums::Update::Story(s) = u {
                if let tl::enums::StoryItem::Item(full) = &s.story {
                    return Ok(full.id);
                }
            }
        }
        anyhow::bail!("repost: id новой сторис не найден")
    }

    async fn set_emoji_status(&self, document_id: Option<i64>) -> Result<()> {
        use grammers_client::tl;
        let emoji = match document_id {
            Some(id) => tl::enums::EmojiStatus::Status(tl::types::EmojiStatus {
                document_id: id,
                until: None,
            }),
            None => tl::enums::EmojiStatus::Empty,
        };
        self.client
            .invoke(&tl::functions::account::UpdateEmojiStatus {
                emoji_status: emoji,
            })
            .await
            .context("emoji status failed")?;
        Ok(())
    }

    async fn default_emoji_statuses(&self) -> Result<Vec<i64>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::account::GetDefaultEmojiStatuses { hash: 0 })
            .await
            .context("emoji statuses failed")?;
        let list = match res {
            tl::enums::account::EmojiStatuses::Statuses(s) => s.statuses,
            _ => vec![],
        };
        let mut out = Vec::new();
        for st in list {
            match st {
                tl::enums::EmojiStatus::Status(s) => out.push(s.document_id),
                tl::enums::EmojiStatus::Collectible(s) => out.push(s.document_id),
                _ => {}
            }
        }
        Ok(out)
    }

    async fn qr_login_start(&self) -> Result<(Vec<u8>, i64)> {
        use grammers_client::tl;
        // api_id/api_hash берём из конфигурации клиента при connect.
        let res = self
            .client
            .invoke(&tl::functions::auth::ExportLoginToken {
                api_id: self.api_id,
                api_hash: self.api_hash.clone(),
                except_ids: vec![],
            })
            .await
            .context("exportLoginToken failed")?;
        match res {
            tl::enums::auth::LoginToken::Token(t) => Ok((t.token, t.expires as i64)),
            tl::enums::auth::LoginToken::MigrateTo(m) => {
                anyhow::bail!("QR выдан другим DC (dc {}): войдите по коду", m.dc_id)
            }
            _ => anyhow::bail!("неожиданный ответ exportLoginToken"),
        }
    }

    async fn qr_login_poll(&self, token: &[u8]) -> Result<Option<i64>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::auth::ImportLoginToken {
                token: token.to_vec(),
            })
            .await;
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{e:?}");
                // Токен истёк/отозван — вызывающий покажет QR заново.
                if msg.contains("AUTH_TOKEN_EXPIRED") || msg.contains("AUTH_TOKEN_INVALID") {
                    anyhow::bail!("QR-код истёк — запросите новый");
                }
                return Err(e).context("importLoginToken failed");
            }
        };
        match res {
            tl::enums::auth::LoginToken::Token(_) => Ok(None),
            tl::enums::auth::LoginToken::Success(s) => {
                let user_id = match &s.authorization {
                    tl::enums::auth::Authorization::Authorization(a) => match &a.user {
                        tl::enums::User::User(u) => u.id,
                        _ => 0,
                    },
                    _ => 0,
                };
                Ok(Some(user_id))
            }
            _ => Ok(None),
        }
    }

    async fn stars_balance(&self) -> Result<(i64, i32)> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::payments::GetStarsStatus {
                ton: false,
                peer: tl::enums::InputPeer::PeerSelf,
            })
            .await
            .context("getStarsStatus failed")?;
        let st = match res {
            tl::enums::payments::StarsStatus::Status(s) => s,
            _ => anyhow::bail!("unexpected stars status"),
        };
        let (amount, nanos) = match &st.balance {
            tl::enums::StarsAmount::Amount(a) => (a.amount, a.nanos),
            tl::enums::StarsAmount::StarsTonAmount(t) => (t.amount, 0),
            _ => (0, 0),
        };
        Ok((amount, nanos))
    }

    async fn participants(
        &self,
        chat_id: i64,
        limit: usize,
    ) -> Result<Vec<super::ParticipantInfo>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        let res = self
            .client
            .invoke(&tl::functions::channels::GetParticipants {
                channel,
                filter: tl::enums::ChannelParticipantsFilter::ChannelParticipantsRecent,
                offset: 0,
                limit: limit.min(200) as i32,
                hash: 0,
            })
            .await
            .context("getParticipants failed (не канал/супергруппа?)")?;
        let (users, parts) = match res {
            tl::enums::channels::ChannelParticipants::Participants(p) => (p.users, p.participants),
            tl::enums::channels::ChannelParticipants::NotModified => (vec![], vec![]),
        };
        let mut meta: std::collections::HashMap<i64, (String, i64)> =
            std::collections::HashMap::new();
        for u in &users {
            if let tl::enums::User::User(u) = u {
                let name = match (&u.first_name, &u.last_name) {
                    (Some(f), Some(l)) => format!("{f} {l}"),
                    (Some(f), None) => f.clone(),
                    (None, Some(l)) => l.clone(),
                    (None, None) => u.username.clone().unwrap_or_else(|| format!("id{}", u.id)),
                };
                meta.insert(u.id, (name, u.access_hash.unwrap_or(0)));
            }
        }
        let mut out = Vec::new();
        for cp in parts {
            let (uid, admin) = match cp {
                tl::enums::ChannelParticipant::Creator(c) => (c.user_id, true),
                tl::enums::ChannelParticipant::Participant(p) => (p.user_id, false),
                tl::enums::ChannelParticipant::Admin(a) => (a.user_id, true),
                _ => continue,
            };
            let (name, hash) = meta
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| (format!("id {uid}"), 0));
            out.push(super::ParticipantInfo {
                id: uid,
                name,
                admin,
                access_hash: hash,
            });
        }
        Ok(out)
    }

    async fn kick_participant(&self, chat_id: i64, user_id: i64) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        // access_hash подтягиваем из списка участников (кэш не храним — один лишний запрос ок).
        let mut access_hash = 0i64;
        for p in self.participants(chat_id, 200).await.unwrap_or_default() {
            if p.id == user_id {
                access_hash = p.access_hash;
                break;
            }
        }
        // Бан на просмотр = исключение из канала (нужны права админа).
        let user = tl::enums::InputPeer::User(tl::types::InputPeerUser {
            user_id,
            access_hash,
        });
        self.client
            .invoke(&tl::functions::channels::EditBanned {
                channel,
                participant: user,
                banned_rights: tl::enums::ChatBannedRights::Rights(tl::types::ChatBannedRights {
                    until_date: 0,
                    view_messages: true,
                    send_messages: false,
                    send_media: false,
                    send_stickers: false,
                    send_gifs: false,
                    send_games: false,
                    send_inline: false,
                    embed_links: false,
                    send_polls: false,
                    change_info: false,
                    invite_users: false,
                    pin_messages: false,
                    manage_topics: false,
                    send_photos: false,
                    send_videos: false,
                    send_roundvideos: false,
                    send_audios: false,
                    send_voices: false,
                    send_docs: false,
                    send_plain: false,
                    edit_rank: false,
                    send_reactions: false,
                    manage_linked_peers: false,
                }),
            })
            .await
            .context("kick failed (нет прав?)")?;
        Ok(())
    }

    async fn hide_peer_stories(&self, peer_id: i64, hidden: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(peer_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::stories::TogglePeerStoriesHidden {
                peer: (&pref).into(),
                hidden,
            })
            .await
            .context("togglePeerStoriesHidden failed")?;
        Ok(())
    }

    async fn edit_story_caption(&self, story_id: i32, caption: &str) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::stories::EditStory {
                peer: tl::enums::InputPeer::PeerSelf,
                id: story_id,
                media: None,
                media_areas: None,
                caption: if caption.is_empty() {
                    None
                } else {
                    Some(caption.to_string())
                },
                entities: None,
                privacy_rules: None,
                music: None,
            })
            .await
            .context("editStory failed")?;
        Ok(())
    }

    async fn story_albums(&self) -> Result<Vec<super::StoryAlbumInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::stories::GetAlbums {
                peer: tl::enums::InputPeer::PeerSelf,
                hash: 0,
            })
            .await
            .context("getAlbums failed")?;
        let albums = match res {
            tl::enums::stories::Albums::Albums(a) => a.albums,
            _ => return Ok(vec![]),
        };
        let mut out = Vec::new();
        for a in albums {
            if let tl::enums::StoryAlbum::Album(al) = a {
                // Число сторис: icon fields не считаем; берём заглушку через длину?
                // Точного count в этом слое нет — покажем 0, UI уточнит при открытии.
                // Лучше: вытащим из icon? Нет. Оставляем 0 — честно, без выдумок.
                // UPD: stories у альбома нет в ответе; count недоступен.
                out.push(super::StoryAlbumInfo {
                    album_id: al.album_id,
                    title: al.title,
                    story_count: 0,
                });
            }
        }
        Ok(out)
    }

    async fn create_album(&self, title: &str, story_ids: &[i32]) -> Result<i32> {
        use grammers_client::tl;
        let title = title.trim();
        if title.is_empty() || story_ids.is_empty() {
            anyhow::bail!("нужны название и хотя бы одна сторис");
        }
        let res = self
            .client
            .invoke(&tl::functions::stories::CreateAlbum {
                peer: tl::enums::InputPeer::PeerSelf,
                title: title.to_string(),
                stories: story_ids.to_vec(),
            })
            .await
            .context("createAlbum failed")?;
        match res {
            tl::enums::StoryAlbum::Album(al) => Ok(al.album_id),
        }
    }

    async fn delete_album(&self, album_id: i32) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::stories::DeleteAlbum {
                peer: tl::enums::InputPeer::PeerSelf,
                album_id,
            })
            .await
            .context("deleteAlbum failed")?;
        Ok(())
    }

    async fn poll_voters(
        &self,
        chat_id: i64,
        message_id: i32,
        option: Option<Vec<u8>>,
    ) -> Result<(i32, Vec<String>)> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let res = self
            .client
            .invoke(&tl::functions::messages::GetPollVotes {
                peer: (&pref).into(),
                id: message_id,
                option,
                offset: None,
                limit: 20,
            })
            .await
            .context("getPollVotes failed")?;
        let list = match res {
            tl::enums::messages::VotesList::List(l) => l,
        };
        let names = users_map(&list.users);
        let mut voters = Vec::new();
        for v in &list.votes {
            if let tl::enums::MessagePeerVote::Vote(vv) = v {
                let pid = peer_to_bot_id(&vv.peer);
                if let Some(n) = names.get(&pid).cloned() {
                    voters.push(n);
                }
            }
        }
        Ok((list.count, voters))
    }

    async fn create_folder(
        &self,
        title: &str,
        peer_ids: &[i64],
        emoticon: Option<String>,
    ) -> Result<i32> {
        use grammers_client::tl;
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("пустое название папки");
        }
        // id выбираем сами: max+1 поверх существующих (сервер принимает клиентский id).
        let existing = self.dialog_filters().await.unwrap_or_default();
        let new_id = existing.iter().map(|f| f.id).max().unwrap_or(0).max(1) + 1;
        let inputs: Vec<tl::enums::InputPeer> = {
            let mut v = Vec::new();
            for pid in peer_ids {
                if let Some(pref) = self.resolve_ref(*pid).await.unwrap_or(None) {
                    v.push((&pref).into());
                }
            }
            v
        };
        let txt = tl::enums::TextWithEntities::Entities(tl::types::TextWithEntities {
            text: title.to_string(),
            entities: vec![],
        });
        self.client
            .invoke(&tl::functions::messages::UpdateDialogFilter {
                id: new_id,
                filter: Some(tl::enums::DialogFilter::Filter(tl::types::DialogFilter {
                    contacts: false,
                    non_contacts: false,
                    groups: false,
                    broadcasts: false,
                    bots: false,
                    exclude_muted: false,
                    exclude_read: false,
                    exclude_archived: false,
                    title_noanimate: false,
                    id: new_id,
                    title: txt,
                    emoticon,
                    color: None,
                    pinned_peers: vec![],
                    include_peers: inputs,
                    exclude_peers: vec![],
                })),
            })
            .await
            .context("createFolder failed")?;
        Ok(new_id)
    }

    async fn rename_folder(&self, id: i32, title: &str) -> Result<()> {
        use grammers_client::tl;
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("пустое название папки");
        }
        // Читаем текущий фильтр, чтобы не потерять состав и настройки.
        let res = self
            .client
            .invoke(&tl::functions::messages::GetDialogFilters {})
            .await
            .context("getDialogFilters failed")?;
        let filters = match res {
            tl::enums::messages::DialogFilters::Filters(f) => f.filters,
        };
        let mut current = None;
        for f in filters {
            if let tl::enums::DialogFilter::Filter(df) = f {
                if df.id == id {
                    current = Some(df);
                    break;
                }
            }
        }
        let mut df = current.context("папка не найдена")?;
        let txt = tl::enums::TextWithEntities::Entities(tl::types::TextWithEntities {
            text: title.to_string(),
            entities: vec![],
        });
        df.title = txt;
        self.client
            .invoke(&tl::functions::messages::UpdateDialogFilter {
                id,
                filter: Some(tl::enums::DialogFilter::Filter(df)),
            })
            .await
            .context("renameFolder failed")?;
        Ok(())
    }

    async fn delete_folder(&self, id: i32) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::messages::UpdateDialogFilter { id, filter: None })
            .await
            .context("deleteFolder failed")?;
        Ok(())
    }

    async fn set_admin(
        &self,
        chat_id: i64,
        user_id: i64,
        access_hash: i64,
        rights: super::AdminRights,
        title: &str,
    ) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        // Без анонимности и права добавлять админов (консервативный набор).
        let admin_rights = tl::types::ChatAdminRights {
            change_info: rights.change_info,
            post_messages: rights.post_messages,
            edit_messages: rights.edit_messages,
            delete_messages: rights.delete_messages,
            ban_users: rights.ban_users,
            invite_users: rights.invite_users,
            pin_messages: rights.pin_messages,
            add_admins: false,
            anonymous: false,
            manage_call: rights.manage_call,
            other: rights.delete_messages,
            manage_topics: rights.manage_topics,
            post_stories: rights.post_stories,
            edit_stories: rights.edit_stories,
            delete_stories: rights.delete_stories,
            manage_direct_messages: rights.manage_direct_messages,
            manage_ranks: false,
            manage_linked_peers: false,
            manage_welcome_messages: false,
        };
        self.client
            .invoke(&tl::functions::channels::EditAdmin {
                channel,
                user_id: tl::enums::InputUser::User(tl::types::InputUser {
                    user_id,
                    access_hash,
                }),
                admin_rights: tl::enums::ChatAdminRights::Rights(admin_rights),
                rank: if !title.is_empty() {
                    Some(title.to_string())
                } else {
                    None
                },
            })
            .await
            .context("editAdmin failed (нет прав?)")?;
        Ok(())
    }

    async fn set_slowmode(&self, chat_id: i64, seconds: i32) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        self.client
            .invoke(&tl::functions::channels::ToggleSlowMode {
                channel,
                seconds: seconds.max(0),
            })
            .await
            .context("slowmode failed (не канал/нет прав?)")?;
        Ok(())
    }

    async fn check_username(&self, username: &str) -> Result<bool> {
        use grammers_client::tl;
        let username = username.trim().trim_start_matches('@').to_string();
        if username.is_empty() {
            anyhow::bail!("пустой username");
        }
        let free = self
            .client
            .invoke(&tl::functions::account::CheckUsername { username })
            .await
            .context("checkUsername failed")?;
        Ok(free)
    }

    async fn toggle_dialog_pin(&self, chat_id: i64, pinned: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        // DialogPeer — отдельная обёртка: peer → InputDialogPeer::Peer.
        let input = match pref.id.kind() {
            grammers_session::types::PeerKind::User
            | grammers_session::types::PeerKind::Chat
            | grammers_session::types::PeerKind::Channel => {
                tl::enums::InputDialogPeer::Peer(tl::types::InputDialogPeer {
                    peer: (&pref).into(),
                })
            }
            _ => anyhow::bail!("не диалог"),
        };
        self.client
            .invoke(&tl::functions::messages::ToggleDialogPin {
                pinned,
                peer: input,
            })
            .await
            .context("toggleDialogPin failed")?;
        Ok(())
    }

    async fn send_album(
        &self,
        chat_id: i64,
        paths: &[std::path::PathBuf],
        caption: &str,
        schedule_at: Option<std::time::SystemTime>,
    ) -> Result<Vec<i32>> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let schedule_ts = match schedule_at {
            Some(t) => Some(
                t.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i32)
                    .unwrap_or(0),
            ),
            None => None,
        };
        // Без отложки — штатный путь grammers; с отложкой — сырой sendMultiMedia
        // (в grammers нет scheduled-альбомов): те же InputMedia, но с schedule_date.
        if schedule_ts.is_none() {
            use grammers_client::media::InputMedia;
            if paths.len() < 2 {
                anyhow::bail!("для альбома нужно 2+ файлов");
            }
            let mut medias = Vec::new();
            for (i, path) in paths.iter().take(10).enumerate() {
                let uploaded = self
                    .client
                    .upload_file(path)
                    .await
                    .with_context(|| format!("upload failed: {}", path.display()))?;
                let is_photo = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
                    .unwrap_or(false);
                // Подпись — только у первого элемента альбома.
                let cap = if i == 0 { caption.trim() } else { "" };
                if is_photo {
                    medias.push(InputMedia::new().caption(cap).photo(uploaded));
                } else {
                    medias.push(InputMedia::new().caption(cap).document(uploaded));
                }
            }
            let sent = self
                .client
                .send_album(pref, medias)
                .await
                .context("sendAlbum failed")?;
            return Ok(sent.into_iter().flatten().map(|m| m.id()).collect());
        }
        // Отложенный альбом: upload + uploadMedia для каждого файла, затем SendMultiMedia.
        use grammers_client::tl;
        let peer_input: tl::enums::InputPeer = (&pref).into();
        let mut single = Vec::new();
        let mut random: i64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(1);
        let mut first = true;
        for path in paths.iter().take(10) {
            let media = self.input_media_for_file(path, peer_input.clone()).await?;
            let (caption_text, entities) = if first {
                first = false;
                let (t, e) = grammers_client::parsers::parse_markdown_message(caption.trim());
                (t, if e.is_empty() { None } else { Some(e) })
            } else {
                (String::new(), None)
            };
            random += 1;
            single.push(tl::enums::InputSingleMedia::Media(
                tl::types::InputSingleMedia {
                    media,
                    random_id: random,
                    message: caption_text,
                    entities,
                },
            ));
        }
        if single.is_empty() {
            anyhow::bail!("нет файлов для альбома");
        }
        let res = self
            .client
            .invoke(&tl::functions::messages::SendMultiMedia {
                silent: false,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                allow_paid_floodskip: false,
                allow_paid_stars: None,
                peer: peer_input,
                reply_to: None,
                schedule_date: schedule_ts,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                multi_media: single,
            })
            .await
            .context("sendAlbum (scheduled) failed")?;
        // Запланированные сообщения не возвращают id сразу — собираем что есть.
        let mut ids = Vec::new();
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => vec![],
        };
        for u in updates {
            let mid = match u {
                tl::enums::Update::NewMessage(m) => match &m.message {
                    tl::enums::Message::Message(mm) => Some(mm.id),
                    _ => None,
                },
                tl::enums::Update::NewChannelMessage(m) => match &m.message {
                    tl::enums::Message::Message(mm) => Some(mm.id),
                    _ => None,
                },
                _ => None,
            };
            if let Some(id) = mid {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    async fn archived_stories(&self) -> Result<Vec<StoryInfo>> {
        use grammers_client::tl;
        let me = self.client.get_me().await?;
        let me_id = me.id().bot_api_dialog_id().unwrap_or(0);
        let me_name = me.full_name();
        let res = self
            .client
            .invoke(&tl::functions::stories::GetStoriesArchive {
                peer: tl::enums::InputPeer::PeerSelf,
                offset_id: 0,
                limit: 50,
            })
            .await
            .context("getStoriesArchive failed")?;
        let (count, items) = match res {
            tl::enums::stories::Stories::Stories(s) => (s.count, s.stories),
        };
        tracing::debug!("archive count={count}");
        let mut out = Vec::new();
        for st in items {
            let full = match st {
                tl::enums::StoryItem::Item(f) => f,
                _ => continue,
            };
            out.push(StoryInfo {
                peer_id: me_id,
                peer_title: if me_name.trim().is_empty() {
                    "Я".to_string()
                } else {
                    me_name.clone()
                },
                story_id: full.id,
                caption: full.caption.clone().unwrap_or_default(),
                has_url: false,
                date: full.date as i64,
                preview_path: None,
            });
        }
        Ok(out)
    }

    async fn album_stories(&self, album_id: i32) -> Result<Vec<StoryInfo>> {
        use grammers_client::tl;
        let me = self.client.get_me().await?;
        let me_id = me.id().bot_api_dialog_id().unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::stories::GetAlbumStories {
                peer: tl::enums::InputPeer::PeerSelf,
                album_id,
                offset: 0,
                limit: 50,
            })
            .await
            .context("getAlbumStories failed")?;
        let stories = match res {
            tl::enums::stories::Stories::Stories(s) => s.stories,
        };
        let mut out = Vec::new();
        for st in stories {
            let full = match st {
                tl::enums::StoryItem::Item(f) => f,
                _ => continue,
            };
            out.push(StoryInfo {
                peer_id: me_id,
                peer_title: "Я".to_string(),
                story_id: full.id,
                caption: full.caption.clone().unwrap_or_default(),
                has_url: false,
                date: full.date as i64,
                preview_path: None,
            });
        }
        Ok(out)
    }

    async fn channel_stats(&self, chat_id: i64) -> Result<super::ChannelStats> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        let res = self
            .client
            .invoke(&tl::functions::stats::GetBroadcastStats {
                dark: false,
                channel,
            })
            .await
            .context("getBroadcastStats failed (не канал/нет прав?)")?;
        let s = match res {
            tl::enums::stats::BroadcastStats::Stats(s) => s,
        };
        let num = |v: &tl::enums::StatsAbsValueAndPrev| -> (f64, f64) {
            match v {
                tl::enums::StatsAbsValueAndPrev::Prev(v) => (v.current, v.previous),
                _ => (0.0, 0.0),
            }
        };
        let pct = |v: &tl::enums::StatsPercentValue| -> f64 {
            match v {
                tl::enums::StatsPercentValue::Value(p) => {
                    if p.total > 0.0 {
                        p.part / p.total * 100.0
                    } else {
                        0.0
                    }
                }
                _ => 0.0,
            }
        };
        let (followers, followers_prev) = num(&s.followers);
        Ok(super::ChannelStats {
            followers: followers as i64,
            followers_prev: followers_prev as i64,
            views_per_post: num(&s.views_per_post).0,
            shares_per_post: num(&s.shares_per_post).0,
            reactions_per_post: num(&s.reactions_per_post).0,
            views_per_story: num(&s.views_per_story).0,
            notifications_pct: pct(&s.enabled_notifications),
        })
    }

    /// Временной ряд для графика аналитики ("followers" | "views").
    /// Async-графы подтягиваются через loadAsyncGraph (до 8 попыток).
    async fn channel_graph(&self, chat_id: i64, kind: &str) -> Result<(Vec<String>, Vec<f64>)> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        let res = self
            .client
            .invoke(&tl::functions::stats::GetBroadcastStats {
                dark: false,
                channel,
            })
            .await
            .context("getBroadcastStats failed (не канал/нет прав?)")?;
        let s = match res {
            tl::enums::stats::BroadcastStats::Stats(s) => s,
        };
        let graph = match kind {
            "views" => s.interactions_graph,
            _ => s.growth_graph,
        };
        let json = Self::resolve_stats_graph(&self.client, graph).await?;
        Ok(parse_stats_json(&json))
    }

    /// Дождаться готового StatsGraph (async → loadAsyncGraph, до ~8 секунд).
    async fn resolve_stats_graph(
        client: &grammers_client::Client,
        graph: grammers_client::tl::enums::StatsGraph,
    ) -> Result<String> {
        use grammers_client::tl;
        let mut g = graph;
        for _ in 0..8 {
            match g {
                tl::enums::StatsGraph::Graph(gr) => {
                    if let tl::enums::DataJson::Json(j) = gr.json {
                        return Ok(j.data);
                    } else {
                        return Ok(String::new());
                    }
                }
                tl::enums::StatsGraph::Async(a) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let res = client
                        .invoke(&tl::functions::stats::LoadAsyncGraph {
                            token: a.token,
                            x: None,
                        })
                        .await
                        .context("loadAsyncGraph failed")?;
                    g = match res {
                        tl::enums::StatsGraph::Graph(gr) => {
                            if let tl::enums::DataJson::Json(j) = gr.json {
                                return Ok(j.data);
                            } else {
                                return Ok(String::new());
                            }
                        }
                        other => other,
                    };
                }
                _ => return Ok(String::new()),
            }
        }
        anyhow::bail!("график не готов (таймаут async-загрузки)");
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
                if std::fs::metadata(&dest)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
                {
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

impl GrammersBackend {
    /// Страница истории старше offset_id (экспорт/прыжок к дате).
    pub async fn history_before(
        &self,
        chat_id: i64,
        offset_id: i32,
        limit: usize,
    ) -> Result<Vec<super::MessageInfo>> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let mut iter = self.client.iter_messages(pref);
        iter = iter.limit(limit.min(200)).offset_id(offset_id);
        let mut out = Vec::new();
        while let Some(m) = iter.next().await? {
            out.push(message_to_info(&m, chat_id));
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Завершить одну сессию (account.resetAuthorization).
    pub async fn terminate_session(&self, hash: i64) -> Result<()> {
        use grammers_client::tl;
        self.client
            .invoke(&tl::functions::account::ResetAuthorization { hash })
            .await
            .context("resetAuthorization failed")?;
        Ok(())
    }

    /// Автоудаление истории чата (messages.setHistoryTTL). 0 — выключить.
    pub async fn set_history_ttl(&self, chat_id: i64, period_secs: i32) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        self.client
            .invoke(&tl::functions::messages::SetHistoryTtl {
                peer: (&pref).into(),
                period: period_secs,
            })
            .await
            .context("setHistoryTTL failed (нужны права?)")?;
        Ok(())
    }

    /// Полное описание канала/группы (channels.getFullChannel).
    pub async fn chat_full(&self, chat_id: i64) -> Result<super::ChatFullInfo> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        let full = self
            .client
            .invoke(&tl::functions::channels::GetFullChannel { channel })
            .await
            .context("getFullChannel failed")?;
        let full_chat = match full {
            tl::enums::messages::ChatFull::Full(f) => f.full_chat,
        };
        match full_chat {
            tl::enums::ChatFull::Full(f) => {
                let count = match &f.participants {
                    tl::enums::ChatParticipants::Participants(p) => {
                        Some(p.participants.len() as i32)
                    }
                    _ => None,
                };
                Ok(super::ChatFullInfo {
                    about: f.about,
                    participants_count: count,
                    ..Default::default()
                })
            }
            tl::enums::ChatFull::ChannelFull(f) => Ok(super::ChatFullInfo {
                about: f.about,
                participants_count: f.participants_count,
                admins_count: f.admins_count,
                online_count: f.online_count,
                linked_chat_id: f.linked_chat_id,
                ttl_period: f.ttl_period,
                antispam: f.antispam,
            }),
            tl::enums::ChatFull::CommunityFull(_) => Ok(super::ChatFullInfo::default()),
        }
    }

    /// Достать пользователя диалога через GetPeerDialogs (там же access_hash).
    async fn dialog_user(&self, chat_id: i64) -> Result<grammers_client::tl::types::User> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let want = pref.id.bare_id_unchecked();
        let res = self
            .client
            .invoke(&tl::functions::messages::GetPeerDialogs {
                peers: vec![tl::enums::InputDialogPeer::Peer(
                    tl::types::InputDialogPeer {
                        peer: (&pref).into(),
                    },
                )],
            })
            .await
            .context("getPeerDialogs failed")?;
        let users = match res {
            tl::enums::messages::PeerDialogs::Dialogs(d) => d.users,
        };
        for u in users {
            if let tl::enums::User::User(t) = u {
                if t.id == want {
                    return Ok(t);
                }
            }
        }
        anyhow::bail!("пользователь не найден в диалоге")
    }

    /// Полное описание собеседника: bio + флаги (users.getFullUser).
    pub async fn peer_full(&self, chat_id: i64) -> Result<super::PeerFullInfo> {
        use grammers_client::tl;
        let user = self.dialog_user(chat_id).await?;
        let access_hash = user.access_hash.unwrap_or(0);
        let full = self
            .client
            .invoke(&tl::functions::users::GetFullUser {
                id: tl::enums::InputUser::User(tl::types::InputUser {
                    user_id: user.id,
                    access_hash,
                }),
            })
            .await
            .context("getFullUser failed")?;
        let payload = match full {
            tl::enums::users::UserFull::Full(f) => f,
        };
        let (about, voice_forbidden) = match payload.full_user {
            tl::enums::UserFull::Full(f) => {
                (f.about.unwrap_or_default(), f.voice_messages_forbidden)
            }
        };
        let (premium, verified, scam, fake) = match payload.users.into_iter().next() {
            Some(tl::enums::User::User(u)) => (u.premium, u.verified, u.scam, u.fake),
            _ => (false, false, false, false),
        };
        Ok(super::PeerFullInfo {
            about,
            premium,
            verified,
            scam,
            fake,
            voice_forbidden,
        })
    }

    /// Кто поставил реакции (messages.getMessageReactionsList).
    pub async fn message_reactions(
        &self,
        chat_id: i64,
        message_id: i32,
    ) -> Result<Vec<super::ReactionVoter>> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let list = self
            .client
            .invoke(&tl::functions::messages::GetMessageReactionsList {
                peer: (&pref).into(),
                id: message_id,
                reaction: None,
                offset: None,
                limit: 100,
            })
            .await
            .context("getMessageReactionsList failed")?;
        let (reactions, users, chats) = match list {
            tl::enums::messages::MessageReactionsList::List(l) => (l.reactions, l.users, l.chats),
        };
        use std::collections::HashMap;
        let mut names: HashMap<i64, String> = HashMap::new();
        for u in users {
            if let tl::enums::User::User(t) = u {
                let name = format!(
                    "{} {}",
                    t.first_name.unwrap_or_default(),
                    t.last_name.unwrap_or_default()
                )
                .trim()
                .to_string();
                names.insert(
                    t.id,
                    if name.is_empty() {
                        format!("id {0}", t.id)
                    } else {
                        name
                    },
                );
            }
        }
        for c in chats {
            let (id, title) = match c {
                tl::enums::Chat::Channel(t) => (t.id, t.title),
                tl::enums::Chat::Chat(t) => (t.id, t.title),
                _ => continue,
            };
            names.insert(id, title);
        }
        Ok(reactions
            .into_iter()
            .filter_map(|r| {
                let r = match r {
                    tl::enums::MessagePeerReaction::Reaction(t) => t,
                };
                let peer_id = match r.peer_id {
                    tl::enums::Peer::User(u) => u.user_id,
                    tl::enums::Peer::Chat(c) => c.chat_id,
                    tl::enums::Peer::Channel(c) => c.channel_id,
                };
                let emoji = match r.reaction {
                    tl::enums::Reaction::Emoji(e) => e.emoticon,
                    tl::enums::Reaction::CustomEmoji(e) => format!("custom:{}", e.document_id),
                    tl::enums::Reaction::Paid => "⭐".into(),
                    _ => "?".into(),
                };
                Some(super::ReactionVoter {
                    user_id: peer_id,
                    name: names
                        .get(&peer_id)
                        .cloned()
                        .unwrap_or_else(|| format!("id {peer_id}")),
                    date: r.date as i64,
                    emoji,
                })
            })
            .collect())
    }

    /// Скачать документ-превью в кэш (стикеры/GIF).
    async fn download_doc_preview(
        &self,
        doc: &grammers_client::tl::enums::Document,
        dest: &std::path::Path,
    ) -> Result<()> {
        use grammers_client::tl;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let media = tl::enums::MessageMedia::Document(tl::types::MessageMediaDocument {
            nopremium: false,
            spoiler: false,
            video: false,
            round: false,
            voice: false,
            document: Some(doc.clone()),
            alt_documents: None,
            video_cover: None,
            video_timestamp: None,
            ttl_seconds: None,
        });
        let gm = grammers_client::media::Media::from_raw(media).context("bad media")?;
        self.client.download_media(&gm, dest).await?;
        Ok(())
    }

    /// Недавние стикеры с превью (messages.getRecentStickers, лимит 24).
    pub async fn recent_stickers(&self) -> Result<Vec<super::StickerInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::messages::GetRecentStickers {
                attached: false,
                hash: 0,
            })
            .await
            .context("getRecentStickers failed")?;
        let stickers = match res {
            tl::enums::messages::RecentStickers::Stickers(r) => r.stickers,
            tl::enums::messages::RecentStickers::NotModified => vec![],
        };
        let dir = super::stickers_cache_dir();
        let mut out = Vec::new();
        for doc in stickers.into_iter().take(24) {
            let tl::enums::Document::Document(d) = doc else {
                continue;
            };
            if d.mime_type == "application/x-tgsticker" {
                // Анимированный TGS превью не рисуем — покажем эмодзи.
                let emoji = d.attributes.iter().find_map(|a| match a {
                    tl::enums::DocumentAttribute::Sticker(s) => Some(s.alt.clone()),
                    _ => None,
                });
                out.push(super::StickerInfo {
                    id: d.id,
                    access_hash: d.access_hash,
                    file_reference: d.file_reference,
                    emoji,
                    preview_path: None,
                });
                continue;
            }
            let dest = dir.join(format!("recent_{}.webp", d.id));
            let preview = if dest.exists() {
                Some(dest)
            } else if self
                .download_doc_preview(&tl::enums::Document::Document(d.clone()), &dest)
                .await
                .is_ok()
                && dest.exists()
            {
                Some(dest)
            } else {
                None
            };
            let emoji = d.attributes.iter().find_map(|a| match a {
                tl::enums::DocumentAttribute::Sticker(s) => Some(s.alt.clone()),
                _ => None,
            });
            out.push(super::StickerInfo {
                id: d.id,
                access_hash: d.access_hash,
                file_reference: d.file_reference,
                emoji,
                preview_path: preview,
            });
        }
        Ok(out)
    }

    /// Отправить существующий документ (стикер/GIF) по ссылке (messages.sendMedia).
    pub async fn send_document_ref(
        &self,
        chat_id: i64,
        doc_id: i64,
        access_hash: i64,
        file_reference: Vec<u8>,
        caption: &str,
        silent: bool,
    ) -> Result<i32> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::messages::SendMedia {
                silent,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: true,
                invert_media: false,
                allow_paid_floodskip: false,
                peer: (&pref).into(),
                reply_to: None,
                media: tl::enums::InputMedia::Document(tl::types::InputMediaDocument {
                    spoiler: false,
                    id: tl::enums::InputDocument::Document(tl::types::InputDocument {
                        id: doc_id,
                        access_hash,
                        file_reference,
                    }),
                    video_cover: None,
                    video_timestamp: None,
                    ttl_seconds: None,
                    query: None,
                }),
                message: caption.to_string(),
                random_id,
                reply_markup: None,
                entities: None,
                schedule_date: None,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
            })
            .await
            .context("sendMedia failed")?;
        let mid = match res {
            tl::enums::Updates::UpdateShortSentMessage(m) => m.id,
            tl::enums::Updates::Updates(u) => u
                .updates
                .iter()
                .find_map(|x| match x {
                    tl::enums::Update::NewMessage(nm) => Some(nm.message.id()),
                    _ => None,
                })
                .unwrap_or(0),
            _ => 0,
        };
        Ok(mid)
    }

    /// Сохранённые GIF с превью-кадром (messages.getSavedGifs, лимит 24).
    pub async fn saved_gifs(&self) -> Result<Vec<super::GifInfo>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::messages::GetSavedGifs { hash: 0 })
            .await
            .context("getSavedGifs failed")?;
        let gifs = match res {
            tl::enums::messages::SavedGifs::Gifs(g) => g.gifs,
            tl::enums::messages::SavedGifs::NotModified => vec![],
        };
        let dir = super::stickers_cache_dir();
        let mut out = Vec::new();
        for doc in gifs.into_iter().take(24) {
            let tl::enums::Document::Document(d) = doc else {
                continue;
            };
            let (mut w, mut h, mut dur) = (0, 0, 0);
            for a in &d.attributes {
                if let tl::enums::DocumentAttribute::Video(v) = a {
                    w = v.w;
                    h = v.h;
                    dur = (v.duration * 1000.0) as i32;
                }
            }
            let dest = dir.join(format!("gif_{}.mp4", d.id));
            if !dest.exists() {
                let _ = self
                    .download_doc_preview(&tl::enums::Document::Document(d.clone()), &dest)
                    .await;
            }
            let poster = dir.join(format!("gif_{}.jpg", d.id));
            if dest.exists() && !poster.exists() {
                if let Some(frame) = crate::media::video::first_frame_builtin(&dest) {
                    crate::media::video::save_poster_jpeg(&frame, &poster);
                }
            }
            out.push(super::GifInfo {
                id: d.id,
                access_hash: d.access_hash,
                file_reference: d.file_reference,
                width: w,
                height: h,
                duration_ms: dur,
                preview_path: if poster.exists() { Some(poster) } else { None },
            });
        }
        Ok(out)
    }

    /// Длительность и waveform голосового из аудиофайла (декодируем rodio).
    fn voice_probe(path: &std::path::Path) -> (i32, Vec<u8>) {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => return (0, vec![0u8; 63]),
        };
        let cursor = std::io::Cursor::new(data);
        let source = match rodio::Decoder::new(cursor) {
            Ok(s) => s,
            Err(_) => return (0, vec![0u8; 63]),
        };
        use rodio::Source;
        let channels = source.channels().get().max(1) as u64;
        let rate = source.sample_rate().get().max(1) as u64;
        let samples: Vec<f32> = source.take((channels * rate * 600) as usize).collect();
        if samples.is_empty() {
            return (0, vec![0u8; 63]);
        }
        let duration = (samples.len() as u64 / (channels * rate)).min(3600) as i32;
        // 63 значения 0..31 по RMS каждого чанка (формат Telegram, сэмплы уже -1..1).
        let chunk = (samples.len() / 63).max(1);
        let mut wave = Vec::with_capacity(63);
        for c in samples.chunks(chunk).take(63) {
            let rms = (c.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / c.len() as f64).sqrt();
            wave.push((rms * 31.0 * 2.2).clamp(0.0, 31.0) as u8);
        }
        while wave.len() < 63 {
            wave.push(0);
        }
        (duration.max(1), wave)
    }

    /// Отправить аудиофайл как голосовое (upload + SendMedia с voice-атрибутом).
    pub async fn send_voice(
        &self,
        chat_id: i64,
        path: &std::path::Path,
        caption: &str,
        silent: bool,
        schedule_at: Option<std::time::SystemTime>,
    ) -> Result<i32> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let uploaded = self.client.upload_file(path).await?;
        let (duration, waveform) = Self::voice_probe(path);
        let mime = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref()
        {
            Some("opus") | Some("ogg") => "audio/ogg",
            Some("m4a") => "audio/mp4",
            Some("mp3") => "audio/mpeg",
            _ => "audio/ogg",
        }
        .to_string();
        let fname = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "voice.ogg".into());
        let _ = fname;
        let file = uploaded.raw;
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::messages::SendMedia {
                silent,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                allow_paid_floodskip: false,
                peer: (&pref).into(),
                reply_to: None,
                media: tl::enums::InputMedia::UploadedDocument(
                    tl::types::InputMediaUploadedDocument {
                        nosound_video: false,
                        force_file: false,
                        spoiler: false,
                        file,
                        thumb: None,
                        mime_type: mime,
                        attributes: vec![
                            tl::enums::DocumentAttribute::Audio(
                                tl::types::DocumentAttributeAudio {
                                    voice: true,
                                    duration,
                                    title: None,
                                    performer: None,
                                    waveform: Some(waveform),
                                },
                            ),
                            tl::enums::DocumentAttribute::Filename(
                                tl::types::DocumentAttributeFilename { file_name: fname },
                            ),
                        ],
                        stickers: None,
                        video_cover: None,
                        video_timestamp: None,
                        ttl_seconds: None,
                    },
                ),
                message: caption.to_string(),
                random_id,
                reply_markup: None,
                entities: None,
                schedule_date: schedule_at.map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i32)
                        .unwrap_or(0)
                }),
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
            })
            .await
            .context("sendMedia (voice) failed")?;
        let mid = match res {
            tl::enums::Updates::UpdateShortSentMessage(m) => m.id,
            tl::enums::Updates::Updates(u) => u
                .updates
                .iter()
                .find_map(|x| match x {
                    tl::enums::Update::NewMessage(nm) => Some(nm.message.id()),
                    _ => None,
                })
                .unwrap_or(0),
            _ => 0,
        };
        Ok(mid)
    }

    /// Люди и группы рядом (contacts.getLocated).
    pub async fn nearby(&self, lat: f64, long: f64) -> Result<Vec<super::NearbyPeer>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::contacts::GetLocated {
                background: false,
                geo_point: tl::enums::InputGeoPoint::Point(tl::types::InputGeoPoint {
                    lat,
                    long,
                    accuracy_radius: None,
                }),
                self_expires: None,
            })
            .await
            .context("getLocated failed")?;
        let (chats, users) = match res {
            tl::enums::Updates::Updates(u) => (u.chats, u.users),
            tl::enums::Updates::Combined(u) => (u.chats, u.users),
            _ => (vec![], vec![]),
        };
        let mut out = Vec::new();
        for u in users {
            if let tl::enums::User::User(t) = u {
                let name = format!(
                    "{} {}",
                    t.first_name.unwrap_or_default(),
                    t.last_name.unwrap_or_default()
                )
                .trim()
                .to_string();
                out.push(super::NearbyPeer {
                    id: t.id,
                    title: if name.is_empty() {
                        format!("id {}", t.id)
                    } else {
                        name
                    },
                    is_user: true,
                });
            }
        }
        for c in chats {
            let (id, title) = match c {
                tl::enums::Chat::Channel(t) => (t.id, t.title),
                tl::enums::Chat::Chat(t) => (t.id, t.title),
                _ => continue,
            };
            out.push(super::NearbyPeer {
                id,
                title,
                is_user: false,
            });
        }
        Ok(out)
    }

    /// Тарифы Stars-подарков (только чтение, payments.getStarsGiftOptions).
    pub async fn star_gift_options(&self, chat_id: i64) -> Result<Vec<super::StarGiftOption>> {
        use grammers_client::tl;
        let user = self.dialog_user(chat_id).await?;
        let options = self
            .client
            .invoke(&tl::functions::payments::GetStarsGiftOptions {
                user_id: Some(tl::enums::InputUser::User(tl::types::InputUser {
                    user_id: user.id,
                    access_hash: user.access_hash.unwrap_or(0),
                })),
            })
            .await
            .context("getStarsGiftOptions failed")?;
        Ok(options
            .into_iter()
            .map(|o| match o {
                tl::enums::StarsGiftOption::Option(t) => super::StarGiftOption {
                    stars: t.stars,
                    currency: t.currency,
                    amount: t.amount,
                },
            })
            .collect())
    }
}

impl GrammersBackend {
    /// Извлечь (salt1, salt2, g, p) из KDF-алгоритма. Только SHA256+PBKDF2-HMAC-SHA512-100000.
    fn kdf_params(
        algo: &grammers_client::tl::enums::PasswordKdfAlgo,
    ) -> Result<(Vec<u8>, Vec<u8>, i32, Vec<u8>)> {
        use grammers_client::tl;
        match algo {
            tl::enums::PasswordKdfAlgo::Sha256Sha256Pbkdf2Hmacsha512iter100000Sha256ModPow(t) => {
                Ok((t.salt1.clone(), t.salt2.clone(), t.g, t.p.clone()))
            }
            _ => anyhow::bail!("неподдерживаемый KDF-алгоритм сервера"),
        }
    }

    /// Построить SRP-проверку пароля (InputCheckPasswordSrp) по данным GetPassword.
    fn srp_check(
        pw: &grammers_client::tl::types::account::Password,
        password: &str,
    ) -> Result<grammers_client::tl::enums::InputCheckPasswordSrp> {
        use grammers_client::tl;
        let algo = pw
            .current_algo
            .as_ref()
            .context("у аккаунта нет пароля — оставьте текущее пустым")?;
        let (salt1, salt2, g, p) = Self::kdf_params(algo)?;
        let srp_b = pw.srp_b.clone().context("нет srp_B")?;
        let srp_id = pw.srp_id.context("нет srp_id")?;
        let a = pw.secure_random.clone();
        let (m1, g_a) = grammers_crypto::two_factor_auth::calculate_2fa(
            &salt1, &salt2, &p, &g, srp_b, a, password,
        );
        Ok(tl::enums::InputCheckPasswordSrp::Srp(
            tl::types::InputCheckPasswordSrp {
                srp_id,
                a: g_a.to_vec(),
                m1: m1.to_vec(),
            },
        ))
    }

    /// Сменить/установить пароль 2FA. После обновления сразу проверяет новый
    /// пароль через GetPasswordSettings — доказательство, что хэш верный.
    pub async fn change_password(
        &self,
        current: &str,
        new: &str,
        hint: &str,
        email: &str,
    ) -> Result<()> {
        use grammers_client::tl;
        if new.len() < 4 {
            anyhow::bail!("новый пароль — минимум 4 символа");
        }
        let pw = self
            .client
            .invoke(&tl::functions::account::GetPassword {})
            .await
            .context("getPassword failed")?;
        let pw = match pw {
            tl::enums::account::Password::Password(p) => p,
        };
        if !pw.has_password && !current.is_empty() {
            anyhow::bail!("у аккаунта нет пароля — поле текущего оставьте пустым");
        }
        let check = if pw.has_password {
            Self::srp_check(&pw, current)?
        } else {
            tl::enums::InputCheckPasswordSrp::InputCheckPasswordEmpty
        };
        let (new_salt1, new_salt2, _, _) = Self::kdf_params(&pw.new_algo)?;
        let new_hash = password_hash_ph2(new.as_bytes(), &new_salt1, &new_salt2);
        self.client
            .invoke(&tl::functions::account::UpdatePasswordSettings {
                password: check,
                new_settings: tl::enums::account::PasswordInputSettings::Settings(
                    tl::types::account::PasswordInputSettings {
                        new_algo: Some(pw.new_algo.clone()),
                        new_password_hash: Some(new_hash),
                        hint: Some(hint.to_string()),
                        email: Some(email.to_string()),
                        new_secure_settings: None,
                    },
                ),
            })
            .await
            .context("updatePasswordSettings failed (неверный текущий пароль?)")?;
        // Проверка: свежие параметры + новый пароль обязаны пройти.
        let pw2 = self
            .client
            .invoke(&tl::functions::account::GetPassword {})
            .await
            .context("getPassword failed")?;
        let pw2 = match pw2 {
            tl::enums::account::Password::Password(p) => p,
        };
        let check2 = Self::srp_check(&pw2, new)?;
        self.client
            .invoke(&tl::functions::account::GetPasswordSettings { password: check2 })
            .await
            .context("проверка нового пароля не прошла — СРОЧНО проверьте вход!")?;
        Ok(())
    }
}

impl GrammersBackend {
    /// Каталог Stars-подарков для отправки (payments.getStarGifts).
    pub async fn gift_catalog(&self) -> Result<Vec<super::StarGiftFull>> {
        use grammers_client::tl;
        let res = self
            .client
            .invoke(&tl::functions::payments::GetStarGifts { hash: 0 })
            .await
            .context("getStarGifts failed")?;
        let gifts = match res {
            tl::enums::payments::StarGifts::Gifts(g) => g.gifts,
            tl::enums::payments::StarGifts::NotModified => vec![],
        };
        Ok(gifts
            .into_iter()
            .filter_map(|g| match g {
                tl::enums::StarGift::Gift(t) => Some(super::StarGiftFull {
                    id: t.id,
                    stars: t.stars,
                    title: t.title.unwrap_or_else(|| format!("подарок {}", t.id)),
                    sold_out: t.sold_out,
                    require_premium: t.require_premium,
                    limited: t.limited,
                    convert_stars: t.convert_stars,
                }),
                // Коллекционные (unique) через обычный invoice не отправить.
                tl::enums::StarGift::Unique(_) => None,
            })
            .collect())
    }

    /// Отправить Stars-подарок: invoice → форма → sendStarsForm.
    pub async fn send_star_gift(
        &self,
        chat_id: i64,
        gift_id: i64,
        hide_name: bool,
        message: &str,
    ) -> Result<String> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let invoice = tl::enums::InputInvoice::StarGift(tl::types::InputInvoiceStarGift {
            hide_name,
            include_upgrade: false,
            peer: (&pref).into(),
            gift_id,
            message: if message.is_empty() {
                None
            } else {
                Some(tl::enums::TextWithEntities::Entities(
                    tl::types::TextWithEntities {
                        text: message.to_string(),
                        entities: vec![],
                    },
                ))
            },
        });
        let form = self
            .client
            .invoke(&tl::functions::payments::GetPaymentForm {
                invoice: invoice.clone(),
                theme_params: None,
            })
            .await
            .context("getPaymentForm failed (нет звёзд? нет подарка?)")?;
        let form_id = form.form_id();
        let result = self
            .client
            .invoke(&tl::functions::payments::SendStarsForm { form_id, invoice })
            .await
            .context("sendStarsForm failed")?;
        match result {
            tl::enums::payments::PaymentResult::Result(_) => {
                Ok(format!("подарок #{gift_id} отправлен"))
            }
            tl::enums::payments::PaymentResult::PaymentVerificationNeeded(_) => {
                anyhow::bail!("нужна верификация платежа в официальном клиенте")
            }
        }
    }
}

impl GrammersBackend {
    /// Видео-кружок: заливка + SendMedia с round_message (dims из первого кадра,
    /// duration 0 — сервер терпит, как в сторис).
    pub async fn send_round_video(
        &self,
        chat_id: i64,
        path: &std::path::Path,
        caption: &str,
        silent: bool,
        schedule_at: Option<std::time::SystemTime>,
    ) -> Result<i32> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let uploaded = self.client.upload_file(path).await?;
        let (w, h) = crate::media::video::first_frame_builtin(path)
            .map(|img| (img.width() as i32, img.height() as i32))
            .unwrap_or((320, 320));
        let fname = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "round.mp4".into());
        let random_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let res = self
            .client
            .invoke(&tl::functions::messages::SendMedia {
                silent,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                allow_paid_floodskip: false,
                peer: (&pref).into(),
                reply_to: None,
                media: tl::enums::InputMedia::UploadedDocument(
                    tl::types::InputMediaUploadedDocument {
                        nosound_video: false,
                        force_file: false,
                        spoiler: false,
                        file: uploaded.raw,
                        thumb: None,
                        mime_type: "video/mp4".to_string(),
                        attributes: vec![
                            tl::enums::DocumentAttribute::Video(
                                tl::types::DocumentAttributeVideo {
                                    round_message: true,
                                    supports_streaming: true,
                                    nosound: false,
                                    duration: 0.0,
                                    w,
                                    h,
                                    preload_prefix_size: None,
                                    video_start_ts: None,
                                    video_codec: None,
                                },
                            ),
                            tl::enums::DocumentAttribute::Filename(
                                tl::types::DocumentAttributeFilename { file_name: fname },
                            ),
                        ],
                        stickers: None,
                        video_cover: None,
                        video_timestamp: None,
                        ttl_seconds: None,
                    },
                ),
                message: caption.to_string(),
                random_id,
                reply_markup: None,
                entities: None,
                schedule_date: schedule_at.map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i32)
                        .unwrap_or(0)
                }),
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
            })
            .await
            .context("sendMedia (round) failed (нужно квадратное видео?)")?;
        let mid = match res {
            tl::enums::Updates::UpdateShortSentMessage(m) => m.id,
            tl::enums::Updates::Updates(u) => u
                .updates
                .iter()
                .find_map(|x| match x {
                    tl::enums::Update::NewMessage(nm) => Some(nm.message.id()),
                    _ => None,
                })
                .unwrap_or(0),
            _ => 0,
        };
        Ok(mid)
    }
}

impl GrammersBackend {
    /// Антиспам канала (channels.toggleAntiSpam).
    pub async fn toggle_antispam(&self, chat_id: i64, enabled: bool) -> Result<()> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let channel: tl::enums::InputChannel = pref.into();
        self.client
            .invoke(&tl::functions::channels::ToggleAntiSpam { channel, enabled })
            .await
            .context("toggleAntiSpam failed (нужны права админа?)")?;
        Ok(())
    }

    /// Жалоба на спам (messages.reportSpam) — помечает пир.
    pub async fn report_spam(&self, chat_id: i64) -> Result<bool> {
        use grammers_client::tl;
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        let res = self
            .client
            .invoke(&tl::functions::messages::ReportSpam {
                peer: (&pref).into(),
            })
            .await
            .context("reportSpam failed")?;
        Ok(res)
    }

    /// Скачивание с реальным прогрессом (параллельные чанки + resume).
    pub async fn download_message_media_with_progress(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
        workers: usize,
        on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<std::path::PathBuf> {
        let pref = self.resolve_ref(chat_id).await?.context("peer not found")?;
        std::fs::create_dir_all(dest_dir).ok();
        let mut msgs = self
            .client
            .get_messages_by_id(pref.clone(), &[message_id])
            .await?;
        let msg = msgs
            .drain(..)
            .flatten()
            .next()
            .context("сообщение не найдено — возможно, удалено")?;
        let media = msg.media().context("у сообщения нет медиа")?;
        let fname = media_file_name(&media, message_id);
        let dest = dest_dir.join(fname);
        crate::media::mtproto::download_parallel(&self.client, &media, &dest, workers, on_progress)
            .await
    }
}

#[async_trait::async_trait]
impl crate::telegram::TelegramService for GrammersBackend {
    async fn dialogs(&self) -> anyhow::Result<Vec<ChatInfo>> {
        GrammersBackend::dialogs(self).await
    }
    async fn history(&self, chat_id: i64, limit: usize) -> anyhow::Result<Vec<MessageInfo>> {
        GrammersBackend::history(self, chat_id, limit).await
    }
    async fn stories(&self) -> anyhow::Result<Vec<StoryInfo>> {
        GrammersBackend::stories(self).await
    }
    async fn download_message_media(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        GrammersBackend::download_message_media(self, chat_id, message_id, dest_dir).await
    }
    async fn download_story_media(
        &self,
        peer_id: i64,
        story_id: i32,
        dest_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        GrammersBackend::download_story_media(self, peer_id, story_id, dest_dir).await
    }
    async fn me(&self) -> anyhow::Result<AccountInfo> {
        GrammersBackend::me(self).await
    }
    async fn send_text(
        &self,
        _chat_id: i64,
        _text: &str,
        _reply_to: Option<i32>,
        _silent: bool,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_text(self, _chat_id, _text, _reply_to, _silent).await
    }
    async fn edit_text(&self, _chat_id: i64, _message_id: i32, _text: &str) -> anyhow::Result<()> {
        GrammersBackend::edit_text(self, _chat_id, _message_id, _text).await
    }
    async fn delete_messages(&self, _chat_id: i64, _message_ids: &[i32]) -> anyhow::Result<()> {
        GrammersBackend::delete_messages(self, _chat_id, _message_ids).await
    }
    async fn react(&self, _chat_id: i64, _message_id: i32, _emoji: &str) -> anyhow::Result<()> {
        GrammersBackend::react(self, _chat_id, _message_id, _emoji).await
    }
    async fn mark_read(&self, _chat_id: i64) -> anyhow::Result<()> {
        GrammersBackend::mark_read(self, _chat_id).await
    }
    async fn send_file(
        &self,
        _chat_id: i64,
        _path: &std::path::Path,
        _caption: &str,
        _silent: bool,
        _schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_file(self, _chat_id, _path, _caption, _silent, _schedule_at).await
    }
    async fn avatar_path(&self, _chat_id: i64) -> anyhow::Result<Option<std::path::PathBuf>> {
        GrammersBackend::avatar_path(self, _chat_id).await
    }
    async fn pin_message(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        GrammersBackend::pin_message(self, _chat_id, _message_id).await
    }
    async fn unpin_message(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        GrammersBackend::unpin_message(self, _chat_id, _message_id).await
    }
    async fn pinned_message(&self, _chat_id: i64) -> anyhow::Result<Option<MessageInfo>> {
        GrammersBackend::pinned_message(self, _chat_id).await
    }
    async fn vote(&self, _chat_id: i64, _message_id: i32, _option: Vec<u8>) -> anyhow::Result<()> {
        GrammersBackend::vote(self, _chat_id, _message_id, _option).await
    }
    async fn forward(
        &self,
        _from_chat: i64,
        _message_ids: &[i32],
        _to_chat: i64,
    ) -> anyhow::Result<usize> {
        GrammersBackend::forward(self, _from_chat, _message_ids, _to_chat).await
    }
    async fn mute(&self, _chat_id: i64, _muted: bool) -> anyhow::Result<()> {
        GrammersBackend::mute(self, _chat_id, _muted).await
    }
    async fn archive(&self, _chat_id: i64, _archived: bool) -> anyhow::Result<()> {
        GrammersBackend::archive(self, _chat_id, _archived).await
    }
    async fn dialog_filters(&self) -> anyhow::Result<Vec<DialogFilterInfo>> {
        GrammersBackend::dialog_filters(self).await
    }
    async fn contacts(&self) -> anyhow::Result<Vec<ContactInfo>> {
        GrammersBackend::contacts(self).await
    }
    async fn send_scheduled(
        &self,
        _chat_id: i64,
        _text: &str,
        _reply_to: Option<i32>,
        _at: std::time::SystemTime,
        _silent: bool,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_scheduled(self, _chat_id, _text, _reply_to, _at, _silent).await
    }
    async fn save_draft(
        &self,
        _chat_id: i64,
        _text: &str,
        _reply_to: Option<i32>,
    ) -> anyhow::Result<()> {
        GrammersBackend::save_draft(self, _chat_id, _text, _reply_to).await
    }
    async fn sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        GrammersBackend::sessions(self).await
    }
    async fn terminate_others(&self) -> anyhow::Result<()> {
        GrammersBackend::terminate_others(self).await
    }
    async fn get_profile(&self) -> anyhow::Result<ProfileInfo> {
        GrammersBackend::get_profile(self).await
    }
    async fn update_profile(
        &self,
        _first_name: &str,
        _last_name: &str,
        _about: &str,
    ) -> anyhow::Result<()> {
        GrammersBackend::update_profile(self, _first_name, _last_name, _about).await
    }
    async fn publish_story(
        &self,
        _path: &std::path::Path,
        _caption: &str,
        _music: Option<std::path::PathBuf>,
    ) -> anyhow::Result<i32> {
        GrammersBackend::publish_story(self, _path, _caption, _music).await
    }
    async fn delete_stories(&self, _story_ids: &[i32]) -> anyhow::Result<()> {
        GrammersBackend::delete_stories(self, _story_ids).await
    }
    async fn forum_topics(&self, _chat_id: i64) -> anyhow::Result<Vec<TopicInfo>> {
        GrammersBackend::forum_topics(self, _chat_id).await
    }
    async fn create_topic(&self, _chat_id: i64, _title: &str) -> anyhow::Result<i32> {
        GrammersBackend::create_topic(self, _chat_id, _title).await
    }
    async fn topic_history(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _limit: usize,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        GrammersBackend::topic_history(self, _chat_id, _topic_id, _limit).await
    }
    async fn send_to_topic(
        &self,
        _chat_id: i64,
        _topic_id: i32,
        _text: &str,
        _silent: bool,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_to_topic(self, _chat_id, _topic_id, _text, _silent).await
    }
    async fn global_search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<(i64, MessageInfo)>> {
        GrammersBackend::global_search(self, _query, _limit).await
    }
    async fn scheduled_history(&self, _chat_id: i64) -> anyhow::Result<Vec<MessageInfo>> {
        GrammersBackend::scheduled_history(self, _chat_id).await
    }
    async fn cancel_scheduled(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        GrammersBackend::cancel_scheduled(self, _chat_id, _message_id).await
    }
    async fn create_channel(
        &self,
        _title: &str,
        _about: &str,
        _broadcast: bool,
    ) -> anyhow::Result<i64> {
        GrammersBackend::create_channel(self, _title, _about, _broadcast).await
    }
    async fn invite_link(&self, _chat_id: i64) -> anyhow::Result<String> {
        GrammersBackend::invite_link(self, _chat_id).await
    }
    async fn block_user(&self, _chat_id: i64, _blocked: bool) -> anyhow::Result<()> {
        GrammersBackend::block_user(self, _chat_id, _blocked).await
    }
    async fn clear_history(&self, _chat_id: i64, _revoke: bool) -> anyhow::Result<()> {
        GrammersBackend::clear_history(self, _chat_id, _revoke).await
    }
    async fn leave_channel(&self, _chat_id: i64) -> anyhow::Result<()> {
        GrammersBackend::leave_channel(self, _chat_id).await
    }
    async fn add_contact(
        &self,
        _phone: &str,
        _first_name: &str,
        _last_name: &str,
    ) -> anyhow::Result<()> {
        GrammersBackend::add_contact(self, _phone, _first_name, _last_name).await
    }
    async fn download_web_photo(
        &self,
        _photo: &WebPreviewPhoto,
        _dest: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        GrammersBackend::download_web_photo(self, _photo, _dest).await
    }
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
        GrammersBackend::create_poll(
            self,
            _chat_id,
            _question,
            _options,
            _quiz,
            _correct,
            _anonymous,
            _multiple_choice,
        )
        .await
    }
    async fn send_geo(&self, _chat_id: i64, _lat: f64, _long: f64) -> anyhow::Result<i32> {
        GrammersBackend::send_geo(self, _chat_id, _lat, _long).await
    }
    async fn story_reply(&self, _peer_id: i64, _story_id: i32, _text: &str) -> anyhow::Result<i32> {
        GrammersBackend::story_reply(self, _peer_id, _story_id, _text).await
    }
    async fn story_views(&self, _story_id: i32) -> anyhow::Result<i32> {
        GrammersBackend::story_views(self, _story_id).await
    }
    async fn enable_stealth(&self) -> anyhow::Result<()> {
        GrammersBackend::enable_stealth(self).await
    }
    async fn story_react(&self, _peer_id: i64, _story_id: i32, _emoji: &str) -> anyhow::Result<()> {
        GrammersBackend::story_react(self, _peer_id, _story_id, _emoji).await
    }
    async fn toggle_story_pin(&self, _story_id: i32, _pinned: bool) -> anyhow::Result<()> {
        GrammersBackend::toggle_story_pin(self, _story_id, _pinned).await
    }
    async fn story_viewers(
        &self,
        _story_id: i32,
        _limit: usize,
    ) -> anyhow::Result<(i32, Vec<StoryViewer>)> {
        GrammersBackend::story_viewers(self, _story_id, _limit).await
    }
    async fn search_media(
        &self,
        _chat_id: i64,
        _kind: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<MessageInfo>> {
        GrammersBackend::search_media(self, _chat_id, _kind, _limit).await
    }
    async fn update_username(&self, _username: &str) -> anyhow::Result<()> {
        GrammersBackend::update_username(self, _username).await
    }
    async fn check_username(&self, _username: &str) -> anyhow::Result<bool> {
        GrammersBackend::check_username(self, _username).await
    }
    async fn toggle_dialog_pin(&self, _chat_id: i64, _pinned: bool) -> anyhow::Result<()> {
        GrammersBackend::toggle_dialog_pin(self, _chat_id, _pinned).await
    }
    async fn send_album(
        &self,
        _chat_id: i64,
        _paths: &[std::path::PathBuf],
        _caption: &str,
        _schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<Vec<i32>> {
        GrammersBackend::send_album(self, _chat_id, _paths, _caption, _schedule_at).await
    }
    async fn archived_stories(&self) -> anyhow::Result<Vec<StoryInfo>> {
        GrammersBackend::archived_stories(self).await
    }
    async fn upload_avatar(&self, _path: &std::path::Path) -> anyhow::Result<()> {
        GrammersBackend::upload_avatar(self, _path).await
    }
    async fn delete_contact(&self, _user_id: i64, _access_hash: i64) -> anyhow::Result<()> {
        GrammersBackend::delete_contact(self, _user_id, _access_hash).await
    }
    async fn blocked_list(&self) -> anyhow::Result<Vec<BlockedInfo>> {
        GrammersBackend::blocked_list(self).await
    }
    async fn share_contact(
        &self,
        _chat_id: i64,
        _phone: &str,
        _first_name: &str,
        _last_name: &str,
    ) -> anyhow::Result<i32> {
        GrammersBackend::share_contact(self, _chat_id, _phone, _first_name, _last_name).await
    }
    async fn stop_poll(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        GrammersBackend::stop_poll(self, _chat_id, _message_id).await
    }
    async fn pinned_list(&self, _chat_id: i64, _limit: usize) -> anyhow::Result<Vec<MessageInfo>> {
        GrammersBackend::pinned_list(self, _chat_id, _limit).await
    }
    async fn revoke_invite(&self, _chat_id: i64, _link: &str) -> anyhow::Result<()> {
        GrammersBackend::revoke_invite(self, _chat_id, _link).await
    }
    async fn participants(
        &self,
        _chat_id: i64,
        _limit: usize,
    ) -> anyhow::Result<Vec<ParticipantInfo>> {
        GrammersBackend::participants(self, _chat_id, _limit).await
    }
    async fn kick_participant(&self, _chat_id: i64, _user_id: i64) -> anyhow::Result<()> {
        GrammersBackend::kick_participant(self, _chat_id, _user_id).await
    }
    async fn hide_peer_stories(&self, _peer_id: i64, _hidden: bool) -> anyhow::Result<()> {
        GrammersBackend::hide_peer_stories(self, _peer_id, _hidden).await
    }
    async fn edit_story_caption(&self, _story_id: i32, _caption: &str) -> anyhow::Result<()> {
        GrammersBackend::edit_story_caption(self, _story_id, _caption).await
    }
    async fn set_admin(
        &self,
        _chat_id: i64,
        _user_id: i64,
        _access_hash: i64,
        _rights: AdminRights,
        _title: &str,
    ) -> anyhow::Result<()> {
        GrammersBackend::set_admin(self, _chat_id, _user_id, _access_hash, _rights, _title).await
    }
    async fn set_slowmode(&self, _chat_id: i64, _seconds: i32) -> anyhow::Result<()> {
        GrammersBackend::set_slowmode(self, _chat_id, _seconds).await
    }
    async fn story_albums(&self) -> anyhow::Result<Vec<StoryAlbumInfo>> {
        GrammersBackend::story_albums(self).await
    }
    async fn create_album(&self, _title: &str, _story_ids: &[i32]) -> anyhow::Result<i32> {
        GrammersBackend::create_album(self, _title, _story_ids).await
    }
    async fn album_stories(&self, _album_id: i32) -> anyhow::Result<Vec<StoryInfo>> {
        GrammersBackend::album_stories(self, _album_id).await
    }
    async fn channel_stats(&self, _chat_id: i64) -> anyhow::Result<ChannelStats> {
        GrammersBackend::channel_stats(self, _chat_id).await
    }
    async fn delete_album(&self, _album_id: i32) -> anyhow::Result<()> {
        GrammersBackend::delete_album(self, _album_id).await
    }
    async fn poll_voters(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _option: Option<Vec<u8>>,
    ) -> anyhow::Result<(i32, Vec<String>)> {
        GrammersBackend::poll_voters(self, _chat_id, _message_id, _option).await
    }
    async fn create_folder(
        &self,
        _title: &str,
        _peer_ids: &[i64],
        _emoticon: Option<String>,
    ) -> anyhow::Result<i32> {
        GrammersBackend::create_folder(self, _title, _peer_ids, _emoticon).await
    }
    async fn rename_folder(&self, _id: i32, _title: &str) -> anyhow::Result<()> {
        GrammersBackend::rename_folder(self, _id, _title).await
    }
    async fn delete_folder(&self, _id: i32) -> anyhow::Result<()> {
        GrammersBackend::delete_folder(self, _id).await
    }
    async fn translate_message(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _to_lang: &str,
    ) -> anyhow::Result<String> {
        GrammersBackend::translate_message(self, _chat_id, _message_id, _to_lang).await
    }
    async fn forward_story(
        &self,
        _peer_id: i64,
        _story_id: i32,
        _caption: &str,
    ) -> anyhow::Result<i32> {
        GrammersBackend::forward_story(self, _peer_id, _story_id, _caption).await
    }
    async fn set_emoji_status(&self, _document_id: Option<i64>) -> anyhow::Result<()> {
        GrammersBackend::set_emoji_status(self, _document_id).await
    }
    async fn default_emoji_statuses(&self) -> anyhow::Result<Vec<i64>> {
        GrammersBackend::default_emoji_statuses(self).await
    }
    async fn qr_login_start(&self) -> anyhow::Result<(Vec<u8>, i64)> {
        GrammersBackend::qr_login_start(self).await
    }
    async fn qr_login_poll(&self, _token: &[u8]) -> anyhow::Result<Option<i64>> {
        GrammersBackend::qr_login_poll(self, _token).await
    }
    async fn stars_balance(&self) -> anyhow::Result<(i64, i32)> {
        GrammersBackend::stars_balance(self).await
    }
    async fn channel_graph(
        &self,
        _chat_id: i64,
        _kind: &str,
    ) -> anyhow::Result<(Vec<String>, Vec<f64>)> {
        GrammersBackend::channel_graph(self, _chat_id, _kind).await
    }
    async fn history_before(
        &self,
        chat_id: i64,
        offset_id: i32,
        limit: usize,
    ) -> anyhow::Result<Vec<super::MessageInfo>> {
        GrammersBackend::history_before(self, chat_id, offset_id, limit).await
    }
    async fn terminate_session(&self, hash: i64) -> anyhow::Result<()> {
        GrammersBackend::terminate_session(self, hash).await
    }
    async fn set_history_ttl(&self, chat_id: i64, period_secs: i32) -> anyhow::Result<()> {
        GrammersBackend::set_history_ttl(self, chat_id, period_secs).await
    }
    async fn chat_full(&self, chat_id: i64) -> anyhow::Result<super::ChatFullInfo> {
        GrammersBackend::chat_full(self, chat_id).await
    }
    async fn peer_full(&self, chat_id: i64) -> anyhow::Result<super::PeerFullInfo> {
        GrammersBackend::peer_full(self, chat_id).await
    }
    async fn message_reactions(
        &self,
        chat_id: i64,
        message_id: i32,
    ) -> anyhow::Result<Vec<super::ReactionVoter>> {
        GrammersBackend::message_reactions(self, chat_id, message_id).await
    }
    async fn recent_stickers(&self) -> anyhow::Result<Vec<super::StickerInfo>> {
        GrammersBackend::recent_stickers(self).await
    }
    async fn send_document_ref(
        &self,
        chat_id: i64,
        doc_id: i64,
        access_hash: i64,
        file_reference: Vec<u8>,
        caption: &str,
        silent: bool,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_document_ref(
            self,
            chat_id,
            doc_id,
            access_hash,
            file_reference,
            caption,
            silent,
        )
        .await
    }
    async fn saved_gifs(&self) -> anyhow::Result<Vec<super::GifInfo>> {
        GrammersBackend::saved_gifs(self).await
    }
    async fn send_voice(
        &self,
        chat_id: i64,
        path: &std::path::Path,
        caption: &str,
        silent: bool,
        schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_voice(self, chat_id, path, caption, silent, schedule_at).await
    }
    async fn nearby(&self, lat: f64, long: f64) -> anyhow::Result<Vec<super::NearbyPeer>> {
        GrammersBackend::nearby(self, lat, long).await
    }
    async fn star_gift_options(&self, chat_id: i64) -> anyhow::Result<Vec<super::StarGiftOption>> {
        GrammersBackend::star_gift_options(self, chat_id).await
    }
    async fn change_password(
        &self,
        current: &str,
        new: &str,
        hint: &str,
        email: &str,
    ) -> anyhow::Result<()> {
        GrammersBackend::change_password(self, current, new, hint, email).await
    }
    async fn gift_catalog(&self) -> anyhow::Result<Vec<super::StarGiftFull>> {
        GrammersBackend::gift_catalog(self).await
    }
    async fn send_star_gift(
        &self,
        chat_id: i64,
        gift_id: i64,
        hide_name: bool,
        message: &str,
    ) -> anyhow::Result<String> {
        GrammersBackend::send_star_gift(self, chat_id, gift_id, hide_name, message).await
    }
    async fn send_round_video(
        &self,
        chat_id: i64,
        path: &std::path::Path,
        caption: &str,
        silent: bool,
        schedule_at: Option<std::time::SystemTime>,
    ) -> anyhow::Result<i32> {
        GrammersBackend::send_round_video(self, chat_id, path, caption, silent, schedule_at).await
    }
    async fn toggle_antispam(&self, chat_id: i64, enabled: bool) -> anyhow::Result<()> {
        GrammersBackend::toggle_antispam(self, chat_id, enabled).await
    }
    async fn report_spam(&self, chat_id: i64) -> anyhow::Result<bool> {
        GrammersBackend::report_spam(self, chat_id).await
    }
    async fn download_message_media_with_progress(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
        workers: usize,
        on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> anyhow::Result<std::path::PathBuf> {
        GrammersBackend::download_message_media_with_progress(
            self,
            chat_id,
            message_id,
            dest_dir,
            workers,
            on_progress,
        )
        .await
    }
    fn is_connected(&self) -> bool {
        GrammersBackend::is_connected(self)
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
    use grammers_client::tl;
    match peer {
        tl::enums::InputPeer::Empty | tl::enums::InputPeer::PeerSelf => None,
        tl::enums::InputPeer::User(u) => Some(u.user_id),
        tl::enums::InputPeer::Chat(c) => Some(-c.chat_id),
        tl::enums::InputPeer::Channel(c) => Some(-(1_000_000_000_000 + c.channel_id)),
        tl::enums::InputPeer::UserFromMessage(u) => Some(u.user_id),
        tl::enums::InputPeer::ChannelFromMessage(c) => Some(-(1_000_000_000_000 + c.channel_id)),
    }
}

fn text_of(t: &grammers_client::tl::enums::TextWithEntities) -> String {
    match t {
        grammers_client::tl::enums::TextWithEntities::Entities(t) => t.text.clone(),
    }
}

/// grammers Message → MessageInfo (единая точка: history, pinned).
/// PH2 хэш нового пароля (байт-в-байт как grammers_crypto::two_factor_auth):
/// PH2(pw, salt1, salt2) = SH(pbkdf2_hmac_sha512(PH1, salt1, 100000), salt2),
/// PH1 = SH(SH(pw, salt1), salt2), SH(data, salt) = SHA256(salt + data + salt).
pub fn password_hash_ph2(password: &[u8], salt1: &[u8], salt2: &[u8]) -> Vec<u8> {
    use hmac::Mac;
    use sha2::{Digest, Sha256, Sha512};
    fn sh(data: &[u8], salt: &[u8]) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(salt);
        h.update(data);
        h.update(salt);
        h.finalize().into()
    }
    let hash1 = {
        let inner = sh(password, salt1);
        sh(&inner, salt2)
    };
    let mut dk = [0u8; 64];
    pbkdf2::pbkdf2::<hmac::Hmac<Sha512>>(&hash1, salt1, 100_000, &mut dk).expect("pbkdf2 failed");
    sh(&dk, salt2).to_vec()
}

fn message_to_info(m: &grammers_client::message::Message, chat_id: i64) -> MessageInfo {
    use grammers_client::media::Media as M;
    use grammers_client::tl;
    let mut kind: Option<String> = None;
    let mut name: Option<String> = None;
    let mut size: Option<i64> = None;
    let mut poll: Option<super::PollInfo> = None;
    let mut geo: Option<(f64, f64)> = None;
    let mut contact: Option<(String, String)> = None;
    let mut web: Option<super::WebPreview> = None;
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
                kind = Some("contact".to_string());
                let full = format!("{} {}", c.first_name(), c.last_name())
                    .trim()
                    .to_string();
                contact = Some((full, c.phone_number().to_string()));
            }
            M::WebPage(w) => {
                if let tl::enums::WebPage::Page(page) = &w.raw.webpage {
                    kind = Some("webpage".to_string());
                    let photo = match &page.photo {
                        Some(tl::enums::Photo::Photo(p)) => Some(super::WebPreviewPhoto {
                            photo_id: p.id,
                            access_hash: p.access_hash,
                            file_reference: p.file_reference.clone(),
                            dc_id: p.dc_id,
                        }),
                        _ => None,
                    };
                    web = Some(super::WebPreview {
                        url: page.url.clone(),
                        title: page.title.clone().unwrap_or_default(),
                        description: page.description.clone().unwrap_or_default(),
                        photo,
                    });
                }
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
        web_preview: web,
        topic_id: None,
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
            super::PollAnswerInfo {
                text,
                option,
                votes,
                chosen,
            }
        })
        .collect();
    super::PollInfo {
        question: text_of(&p.question()),
        closed: p.closed(),
        quiz: p.is_quiz(),
        total_voters: p.total_voters(),
        answers,
        solution: p.raw_results.solution.clone(),
    }
}

/// Самый большой скачиваемый тамб (PhotoSize::Size) для video-preview.
fn largest_thumb(
    thumbs: &[grammers_client::media::PhotoSize],
) -> Option<grammers_client::media::PhotoSize> {
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

/// user_id → отображаемое имя из ответа с пользователями (топики/запланированные).
fn users_map(users: &[grammers_client::tl::enums::User]) -> std::collections::HashMap<i64, String> {
    use grammers_client::tl::enums::User as U;
    let mut out = std::collections::HashMap::new();
    for u in users {
        if let U::User(u) = u {
            let name = match (&u.first_name, &u.last_name) {
                (Some(f), Some(l)) => format!("{f} {l}"),
                (Some(f), None) => f.clone(),
                (None, Some(l)) => l.clone(),
                (None, None) => u.username.clone().unwrap_or_else(|| format!("id{}", u.id)),
            };
            out.insert(u.id, name);
        }
    }
    out
}

/// Сырое `tl::enums::Message` → `MessageInfo` (топики, запланированные):
/// grammers-обёртки там нет, маппим вручную. Отправитель — по карте users
/// (её нет в Slice — тогда только «вы»/id), медиа — метки + имена
/// (скачивание всё равно идёт по id через `get_messages_by_id`).
fn raw_to_info(
    m: &grammers_client::tl::enums::Message,
    chat_id: i64,
    topic: Option<i32>,
    users: &std::collections::HashMap<i64, String>,
) -> Option<MessageInfo> {
    use grammers_client::tl;
    let mm = match m {
        tl::enums::Message::Message(m) => m,
        _ => return None,
    };
    let (outgoing, sender) = match &mm.from_id {
        Some(p) => {
            let id = peer_to_bot_id(p);
            let name = users.get(&id).cloned().or_else(|| {
                mm.post_author.clone().or_else(|| {
                    if mm.out {
                        Some("вы".to_string())
                    } else {
                        None
                    }
                })
            });
            (mm.out, name)
        }
        None => (mm.out, mm.post_author.clone()),
    };
    let mut kind: Option<String> = None;
    let mut name: Option<String> = None;
    let mut poll: Option<super::PollInfo> = None;
    let mut geo: Option<(f64, f64)> = None;
    let mut contact: Option<(String, String)> = None;
    let mut web: Option<super::WebPreview> = None;
    match &mm.media {
        None => {}
        Some(media) => match media {
            tl::enums::MessageMedia::Photo(_) => {
                kind = Some("photo".to_string());
                name = Some(format!("photo_{}.jpg", mm.id));
            }
            tl::enums::MessageMedia::Document(d) => {
                let mut fname = format!("doc_{}", mm.id);
                let mut k = "document";
                if let Some(doc) = &d.document {
                    if let tl::enums::Document::Document(dd) = doc {
                        for a in &dd.attributes {
                            match a {
                                tl::enums::DocumentAttribute::Filename(f) => {
                                    fname = f.file_name.clone();
                                }
                                tl::enums::DocumentAttribute::Sticker(_) => {
                                    k = "sticker";
                                }
                                _ => {}
                            }
                        }
                        // Стикеры уже покрыты именем sticker_<id>.webp для превью.
                        if k == "sticker" {
                            fname = format!("sticker_{}.webp", mm.id);
                        }
                        if dd.mime_type.starts_with("video") {
                            k = "video";
                        } else if dd.mime_type.starts_with("audio") {
                            k = "audio";
                        }
                    }
                }
                kind = Some(k.to_string());
                name = Some(fname);
            }
            tl::enums::MessageMedia::Poll(p) => {
                if let tl::enums::Poll::Poll(raw) = &p.poll {
                    let question = text_of(&raw.question);
                    let mut stat: std::collections::HashMap<Vec<u8>, (Option<i32>, bool)> =
                        std::collections::HashMap::new();
                    let mut total_voters = None;
                    let mut solution = None;
                    if let tl::enums::PollResults::Results(rr) = &p.results {
                        total_voters = rr.total_voters;
                        solution = rr.solution.clone();
                        for v in rr.results.iter().flatten() {
                            if let tl::enums::PollAnswerVoters::Voters(v) = v {
                                stat.insert(v.option.clone(), (v.voters, v.chosen));
                            }
                        }
                    }
                    let answers = raw
                        .answers
                        .iter()
                        .map(|a| match a {
                            tl::enums::PollAnswer::Answer(t) => {
                                let (votes, chosen) =
                                    stat.get(&t.option).copied().unwrap_or((None, false));
                                super::PollAnswerInfo {
                                    text: text_of(&t.text),
                                    option: t.option.clone(),
                                    votes,
                                    chosen,
                                }
                            }
                            _ => super::PollAnswerInfo {
                                text: "?".to_string(),
                                option: vec![],
                                votes: None,
                                chosen: false,
                            },
                        })
                        .collect();
                    poll = Some(super::PollInfo {
                        question,
                        closed: raw.closed,
                        quiz: raw.quiz,
                        total_voters,
                        answers,
                        solution,
                    });
                }
            }
            tl::enums::MessageMedia::Geo(g) => {
                if let tl::enums::GeoPoint::Point(pt) = &g.geo {
                    kind = Some("geo".to_string());
                    geo = Some((pt.lat, pt.long));
                }
            }
            tl::enums::MessageMedia::Contact(c) => {
                kind = Some("contact".to_string());
                contact = Some((
                    format!("{} {}", c.first_name, c.last_name)
                        .trim()
                        .to_string(),
                    c.phone_number.clone(),
                ));
            }
            tl::enums::MessageMedia::WebPage(w) => {
                if let tl::enums::WebPage::Page(page) = &w.webpage {
                    let photo = match &page.photo {
                        Some(tl::enums::Photo::Photo(p)) => Some(super::WebPreviewPhoto {
                            photo_id: p.id,
                            access_hash: p.access_hash,
                            file_reference: p.file_reference.clone(),
                            dc_id: p.dc_id,
                        }),
                        _ => None,
                    };
                    web = Some(super::WebPreview {
                        url: page.url.clone(),
                        title: page.title.clone().unwrap_or_default(),
                        description: page.description.clone().unwrap_or_default(),
                        photo,
                    });
                }
            }
            _ => {
                kind = Some("media".to_string());
                name = Some(format!("media_{}", mm.id));
            }
        },
    }
    let reactions = mm.reactions.as_ref().map(|r| match r {
        tl::enums::MessageReactions::Reactions(rr) => rr
            .results
            .iter()
            .map(|c| match c {
                tl::enums::ReactionCount::Count(cc) => cc.count,
                _ => 0,
            })
            .sum(),
        _ => 0,
    });
    Some(MessageInfo {
        id: mm.id,
        chat_id,
        text: mm.message.clone(),
        date: mm.date as i64,
        has_media: kind.is_some(),
        media_kind: kind,
        file_name: name,
        file_size: None,
        outgoing,
        sender,
        edited: mm.edit_date.is_some(),
        reactions,
        views: mm.views,
        forwards: mm.forwards,
        poll,
        geo,
        contact,
        web_preview: web,
        topic_id: topic,
    })
}

/// Разобрать DataJSON графика статистики Telegram в (подписи оси X, значения Y).
/// Формат: {"columns":[["x",t1,...],["y0",v...],...],"types":{"x":"x","y0":"line",...}}.
/// Берём первую серию с type line/area/bar; подписи дат — dd.MM.
pub fn parse_stats_json(data: &str) -> (Vec<String>, Vec<f64>) {
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return (vec![], vec![]),
    };
    let columns = match v.get("columns").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return (vec![], vec![]),
    };
    let types = v.get("types");
    let mut x: Vec<i64> = vec![];
    let mut y: Vec<f64> = vec![];
    for col in columns {
        let arr = match col.as_array() {
            Some(a) if !a.is_empty() => a,
            _ => continue,
        };
        let name = match arr[0].as_str() {
            Some(n) => n,
            None => continue,
        };
        if name == "x" {
            x = arr[1..].iter().filter_map(|v| v.as_i64()).collect();
            continue;
        }
        if y.is_empty() {
            let is_series = types
                .and_then(|t| t.get(name))
                .and_then(|t| t.as_str())
                .map(|t| matches!(t, "line" | "area" | "bar"))
                .unwrap_or(true);
            if is_series {
                y = arr[1..]
                    .iter()
                    .filter_map(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)))
                    .collect();
            }
        }
    }
    if x.is_empty() || y.is_empty() {
        return (vec![], vec![]);
    }
    // Синхронизируем длины и прореживаем до 180 точек.
    let n = x.len().min(y.len());
    let (mut xs, mut ys) = (x[..n].to_vec(), y[..n].to_vec());
    if n > 180 {
        let step = (n as f64 / 180.0).ceil() as usize;
        let step = step.max(1);
        xs = xs.into_iter().step_by(step).collect();
        ys = ys.into_iter().step_by(step).collect();
    }
    let labels = xs
        .iter()
        .map(|t| {
            chrono::DateTime::from_timestamp(*t, 0)
                .map(|d| d.format("%d.%m").to_string())
                .unwrap_or_default()
        })
        .collect();
    (labels, ys)
}

#[cfg(test)]
mod stats_tests {
    use super::parse_stats_json;

    #[test]
    fn parses_line_series() {
        let data = r##"{"columns":[["x",1700000000,1700086400],["y0",10,25]],"types":{"x":"x","y0":"line"},"names":{"y0":"Followers"},"colors":{"y0":"#ff0000"}}"##;
        let (labels, vals) = parse_stats_json(data);
        assert_eq!(vals, vec![10.0, 25.0]);
        assert_eq!(labels.len(), 2);
        assert!(labels[0].contains('.'));
    }

    #[test]
    fn skips_non_series_and_garbage() {
        let (l, v) = parse_stats_json("not json");
        assert!(l.is_empty() && v.is_empty());
        let data = r##"{"columns":[["x",1],["total",5]],"types":{"x":"x","total":"pie"}}"##;
        let (l2, v2) = parse_stats_json(data);
        // "pie" не серия — пусто.
        assert!(l2.is_empty() && v2.is_empty());
    }

    #[test]
    fn downsamples_long_series() {
        let xs: Vec<String> = (0..500).map(|i| i.to_string()).collect();
        let ys: Vec<String> = (0..500).map(|i| (i % 100).to_string()).collect();
        let data = [
            "{\"columns\":[[\"x\",",
            &xs.join(","),
            "],[\"y0\",",
            &ys.join(","),
            "]],\"types\":{\"x\":\"x\",\"y0\":\"line\"}}",
        ]
        .concat();
        let (l, v) = parse_stats_json(&data);
        assert!(l.len() <= 180 && v.len() == l.len() && !v.is_empty());
    }

    #[test]
    fn password_ph2_matches_independent_python_vector() {
        // Вектор посчитан независимым Python (hashlib): см. историю разработки.
        let salt1: Vec<u8> = (0..32).collect();
        let salt2: Vec<u8> = (0..32).map(|i| 255 - i).collect();
        let got = super::password_hash_ph2(b"test-password-123", &salt1, &salt2);
        assert_eq!(
            hex::encode(&got),
            "342283160858b0a3c8c7a91e1f02e44ab80a8246e693b4bfd970494b5ba84c4b"
        );
    }

    #[test]
    fn password_ph2_is_deterministic_and_sensitive() {
        let s1 = vec![1u8; 32];
        let s2 = vec![2u8; 32];
        let a = super::password_hash_ph2(b"pw", &s1, &s2);
        let b = super::password_hash_ph2(b"pw", &s1, &s2);
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        // Чувствительность к каждому входу.
        assert_ne!(a, super::password_hash_ph2(b"pw!", &s1, &s2));
        assert_ne!(a, super::password_hash_ph2(b"pw", &vec![3u8; 32], &s2));
        assert_ne!(a, super::password_hash_ph2(b"pw", &s1, &vec![4u8; 32]));
        // Пустой пароль — валидный 32-байтный хэш (сервер решает, принимать ли).
        assert_eq!(super::password_hash_ph2(b"", &s1, &s2).len(), 32);
    }
}
