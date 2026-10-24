//! Android-мост (JNI): JSON-протокол поверх [`crate::telegram::TelegramService`].
//!
//! Kotlin вызывает один метод `Core.nativeCall(cmd_json): String`.
//! Ответ всегда `{"ok":true,...}` или `{"ok":false,"error":"..."}`.
//! Чистая логика (`core_call`) не зависит от JNI и покрыта тестами на хосте.
//!
//! Копирование из чатов с защитой от копирования: бэкенд по контракту
//! игнорирует флаг `noforwards` (см. `download_message_media`), поэтому мост
//! просто отдаёт текст/медиа — запрет действует только в официальном клиенте.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use serde_json::{json, Value};

/// Состояние моста: сервис (после входа) + каталог файлов приложения.
pub struct BridgeCore {
    tg: Option<Arc<dyn crate::telegram::TelegramService>>,
    dir: PathBuf,
}

impl BridgeCore {
    fn new() -> Self {
        Self {
            tg: None,
            dir: std::env::temp_dir().join("telecomfort"),
        }
    }

    fn tg(&self) -> Result<Arc<dyn crate::telegram::TelegramService>> {
        self.tg
            .clone()
            .context("not logged in — вызовите init/code_signin")
    }
}

static CORE: OnceLock<Mutex<BridgeCore>> = OnceLock::new();
static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn core() -> std::sync::MutexGuard<'static, BridgeCore> {
    CORE.get_or_init(|| Mutex::new(BridgeCore::new()))
        .lock()
        .unwrap()
}

fn rt() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

// --- helpers ---------------------------------------------------------------
fn s(v: &Value, k: &str) -> Result<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
        .with_context(|| format!("нет строкового поля {k}"))
}

fn s_opt(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string())
}

fn i64f(v: &Value, k: &str) -> Result<i64> {
    v.get(k)
        .and_then(|x| x.as_i64())
        .with_context(|| format!("нет int-поля {k}"))
}

fn i32f(v: &Value, k: &str) -> Result<i32> {
    Ok(i64f(v, k)? as i32)
}

fn boolf(v: &Value, k: &str) -> Result<bool> {
    v.get(k)
        .and_then(|x| x.as_bool())
        .with_context(|| format!("нет bool-поля {k}"))
}

fn opt_i32(v: &Value, k: &str) -> Option<i32> {
    v.get(k).and_then(|x| x.as_i64()).map(|x| x as i32)
}

fn limit(v: &Value) -> usize {
    v.get("limit").and_then(|x| x.as_u64()).unwrap_or(50) as usize
}

fn systime_secs(secs: u64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
}

fn b64(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn b64dec(s: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .context("bad base64")
}

fn is_image_path(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()),
        Some(ref e) if matches!(e.as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif")
    )
}

/// Рекурсивно собрать картинки (глубина ≤ 2) до `limit` штук.
fn collect_image_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        if out.len() >= limit {
            return;
        }
        let p = entry.path();
        if p.is_dir() {
            collect_image_files(&p, out, limit);
        } else if is_image_path(&p) {
            out.push(p);
        }
    }
}

// --- entry -----------------------------------------------------------------
/// Выполнить команду моста, вернуть JSON-ответ строкой.
pub fn core_call(req_json: &str) -> String {
    match core_call_inner(req_json) {
        Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| r#"{"ok":false}"#.into()),
        Err(e) => serde_json::to_string(&json!({"ok": false, "error": format!("{e:#}")}))
            .unwrap_or_else(|_| r#"{"ok":false}"#.into()),
    }
}

fn core_call_inner(req_json: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(req_json).context("bad json")?;
    let op = s(&v, "op")?;
    let out = rt().block_on(dispatch(&op, &v))?;
    Ok(match out {
        Value::Null => json!({"ok": true}),
        Value::Object(_) => {
            let mut o = out.as_object().cloned().unwrap_or_default();
            o.insert("ok".into(), Value::Bool(true));
            Value::Object(o)
        }
        other => json!({"ok": true, "result": other}),
    })
}

// --- dispatch --------------------------------------------------------------
async fn dispatch(op: &str, v: &Value) -> Result<Value> {
    match op {
        // -- вход --
        "init" => {
            let dir = PathBuf::from(s(v, "dir")?);
            std::fs::create_dir_all(&dir).ok();
            let api_id: i32 = v.get("api_id").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let api_hash = s(v, "api_hash")?;
            let proxy = s_opt(v, "proxy").filter(|p| !p.is_empty());
            let session = dir.join("session_android.session");
            let backend = crate::telegram::backend_grammers::GrammersBackend::connect_with_proxy(
                &session, api_id, &api_hash, proxy,
            )
            .await?;
            let authorized = backend.is_authorized().await.unwrap_or(false);
            let mut c = core();
            c.dir = dir;
            c.tg = Some(Arc::new(backend));
            Ok(json!({"authorized": authorized}))
        }
        "use_mock" => {
            let mut c = core();
            c.tg = Some(Arc::new(crate::telegram::MockTelegram));
            Ok(json!({"authorized": true}))
        }
        "request_code" => {
            let tg = core().tg()?;
            Ok(json!({"key": tg.request_code(&s(v, "phone")?).await?}))
        }
        "sign_in_code" => {
            let tg = core().tg()?;
            match tg.sign_in_code(&s(v, "key")?, &s(v, "code")?).await {
                Ok(()) => Ok(json!({"status": "ok"})),
                Err(e) if format!("{e:#}").contains("PASSWORD_REQUIRED") => {
                    Ok(json!({"status": "password_required"}))
                }
                Err(e) => anyhow::bail!("{e:#}"),
            }
        }
        "sign_in_password" => {
            let tg = core().tg()?;
            tg.sign_in_password(&s(v, "key")?, &s(v, "password")?)
                .await?;
            Ok(json!({"status": "ok"}))
        }
        "qr_login_start" => {
            let tg = core().tg()?;
            let (token, expires) = tg.qr_login_start().await?;
            Ok(json!({"token_b64": b64(&token), "expires": expires}))
        }
        "qr_poll" => {
            let tg = core().tg()?;
            let token = b64dec(&s(v, "token_b64")?)?;
            match tg.qr_login_poll(&token).await? {
                Some(uid) => Ok(json!({"status": "ok", "uid": uid})),
                None => Ok(json!({"status": "wait"})),
            }
        }
        "password_hint" => {
            let tg = core().tg()?;
            Ok(json!({"hint": tg.password_hint().await?}))
        }
        "me" => {
            let tg = core().tg()?;
            Ok(json!(tg.me().await?))
        }
        // -- диалоги/история --
        "dialogs" => {
            let tg = core().tg()?;
            Ok(json!(tg.dialogs().await?))
        }
        "history" => {
            let tg = core().tg()?;
            Ok(json!(tg.history(i64f(v, "chat")?, limit(v)).await?))
        }
        "history_before" => {
            let tg = core().tg()?;
            Ok(json!(
                tg.history_before(i64f(v, "chat")?, i32f(v, "offset_id")?, limit(v))
                    .await?
            ))
        }
        "history_at_date" => {
            let tg = core().tg()?;
            let (items, reached) = tg
                .history_at_date(i64f(v, "chat")?, i64f(v, "date")?, limit(v))
                .await?;
            Ok(json!({"items": items, "reached_start": reached}))
        }
        // -- отправка --
        "send_text" => {
            let tg = core().tg()?;
            let id = tg
                .send_text(
                    i64f(v, "chat")?,
                    &s(v, "text")?,
                    opt_i32(v, "reply_to"),
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "send_file" => {
            let tg = core().tg()?;
            let id = tg
                .send_file(
                    i64f(v, "chat")?,
                    &PathBuf::from(s(v, "path")?),
                    &s_opt(v, "caption").unwrap_or_default(),
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                    v.get("schedule_at")
                        .and_then(|x| x.as_u64())
                        .map(systime_secs),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "send_album" => {
            let tg = core().tg()?;
            let paths: Vec<PathBuf> = v
                .get("paths")
                .and_then(|x| x.as_array())
                .context("нет paths[]")?
                .iter()
                .filter_map(|x| x.as_str().map(PathBuf::from))
                .collect();
            let ids = tg
                .send_album(
                    i64f(v, "chat")?,
                    &paths,
                    &s_opt(v, "caption").unwrap_or_default(),
                    v.get("schedule_at")
                        .and_then(|x| x.as_u64())
                        .map(systime_secs),
                )
                .await?;
            Ok(json!({"ids": ids}))
        }
        "send_voice" => {
            let tg = core().tg()?;
            let id = tg
                .send_voice(
                    i64f(v, "chat")?,
                    &PathBuf::from(s(v, "path")?),
                    &s_opt(v, "caption").unwrap_or_default(),
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                    v.get("schedule_at")
                        .and_then(|x| x.as_u64())
                        .map(systime_secs),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "send_round" => {
            let tg = core().tg()?;
            let id = tg
                .send_round_video(
                    i64f(v, "chat")?,
                    &PathBuf::from(s(v, "path")?),
                    &s_opt(v, "caption").unwrap_or_default(),
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                    v.get("schedule_at")
                        .and_then(|x| x.as_u64())
                        .map(systime_secs),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "send_doc_ref" => {
            let tg = core().tg()?;
            let id = tg
                .send_document_ref(
                    i64f(v, "chat")?,
                    i64f(v, "doc_id")?,
                    i64f(v, "access_hash")?,
                    b64dec(&s(v, "file_ref")?)?,
                    &s_opt(v, "caption").unwrap_or_default(),
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "send_scheduled" => {
            let tg = core().tg()?;
            let id = tg
                .send_scheduled(
                    i64f(v, "chat")?,
                    &s(v, "text")?,
                    opt_i32(v, "reply_to"),
                    systime_secs(v.get("at").and_then(|x| x.as_u64()).context("нет at")?),
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        // -- управление сообщениями --
        "delete_msg" => {
            let tg = core().tg()?;
            tg.delete_messages(i64f(v, "chat")?, &[i32f(v, "id")?])
                .await?;
            Ok(json!({}))
        }
        "edit_text" => {
            let tg = core().tg()?;
            tg.edit_text(i64f(v, "chat")?, i32f(v, "id")?, &s(v, "text")?)
                .await?;
            Ok(json!({}))
        }
        "forward" => {
            let tg = core().tg()?;
            let ids: Vec<i32> = v
                .get("ids")
                .and_then(|x| x.as_array())
                .context("нет ids[]")?
                .iter()
                .filter_map(|x| x.as_i64().map(|n| n as i32))
                .collect();
            let n = tg.forward(i64f(v, "from")?, &ids, i64f(v, "to")?).await?;
            Ok(json!({"count": n}))
        }
        "toggle_pin" => {
            let tg = core().tg()?;
            // pin_message(chat, id) — переключатель через pinned_list? Дергаем pin напрямую.
            tg.pin_message(i64f(v, "chat")?, i32f(v, "id")?).await?;
            Ok(json!({}))
        }
        "pinned_one" => {
            let tg = core().tg()?;
            Ok(json!(tg.pinned_message(i64f(v, "chat")?).await?))
        }
        "pinned_list" => {
            let tg = core().tg()?;
            Ok(json!(tg.pinned_list(i64f(v, "chat")?, limit(v)).await?))
        }
        "mark_read" => {
            let tg = core().tg()?;
            tg.mark_read(i64f(v, "chat")?).await?;
            Ok(json!({}))
        }
        "mute" => {
            let tg = core().tg()?;
            tg.mute(i64f(v, "chat")?, boolf(v, "muted")?).await?;
            Ok(json!({}))
        }
        "archive" => {
            let tg = core().tg()?;
            tg.archive(i64f(v, "chat")?, boolf(v, "archived")?).await?;
            Ok(json!({}))
        }
        "pin_dialog" => {
            let tg = core().tg()?;
            tg.toggle_dialog_pin(i64f(v, "chat")?, boolf(v, "pinned")?)
                .await?;
            Ok(json!({}))
        }
        "clear_history" => {
            let tg = core().tg()?;
            tg.clear_history(i64f(v, "chat")?, boolf(v, "revoke")?)
                .await?;
            Ok(json!({}))
        }
        "set_ttl" => {
            let tg = core().tg()?;
            tg.set_history_ttl(i64f(v, "chat")?, i32f(v, "secs")?)
                .await?;
            Ok(json!({}))
        }
        "report_spam" => {
            let tg = core().tg()?;
            Ok(json!({"accepted": tg.report_spam(i64f(v, "chat")?).await?}))
        }
        "toggle_antispam" => {
            let tg = core().tg()?;
            tg.toggle_antispam(i64f(v, "chat")?, boolf(v, "enabled")?)
                .await?;
            let info = tg.chat_full(i64f(v, "chat")?).await?;
            Ok(json!(info))
        }
        "set_slowmode" => {
            let tg = core().tg()?;
            tg.set_slowmode(i64f(v, "chat")?, i32f(v, "secs")?).await?;
            Ok(json!({}))
        }
        // -- папки --
        "create_folder" => {
            let tg = core().tg()?;
            let peers: Vec<i64> = v
                .get("peers")
                .and_then(|x| x.as_array())
                .context("нет peers[]")?
                .iter()
                .filter_map(|x| x.as_i64())
                .collect();
            let id = tg
                .create_folder(&s(v, "title")?, &peers, s_opt(v, "emoticon"))
                .await?;
            Ok(json!({"id": id}))
        }
        "rename_folder" => {
            let tg = core().tg()?;
            tg.rename_folder(i32f(v, "id")?, &s(v, "title")?).await?;
            Ok(json!({}))
        }
        "delete_folder" => {
            let tg = core().tg()?;
            tg.delete_folder(i32f(v, "id")?).await?;
            Ok(json!({}))
        }
        // -- запланированные/черновики --
        "scheduled_list" => {
            let tg = core().tg()?;
            Ok(json!(tg.scheduled_history(i64f(v, "chat")?).await?))
        }
        "cancel_scheduled" => {
            let tg = core().tg()?;
            tg.cancel_scheduled(i64f(v, "chat")?, i32f(v, "id")?)
                .await?;
            Ok(json!({}))
        }
        "save_draft" => {
            let tg = core().tg()?;
            tg.save_draft(i64f(v, "chat")?, &s(v, "text")?, opt_i32(v, "reply_to"))
                .await?;
            Ok(json!({}))
        }
        // -- опросы --
        "create_poll" => {
            let tg = core().tg()?;
            let options: Vec<String> = v
                .get("options")
                .and_then(|x| x.as_array())
                .context("нет options[]")?
                .iter()
                .filter_map(|x| x.as_str().map(|o| o.to_string()))
                .collect();
            let id = tg
                .create_poll(
                    i64f(v, "chat")?,
                    &s(v, "question")?,
                    &options,
                    boolf(v, "quiz")?,
                    opt_i32(v, "correct").map(|n| n as usize),
                    v.get("anonymous").and_then(|x| x.as_bool()).unwrap_or(true),
                    v.get("multiple").and_then(|x| x.as_bool()).unwrap_or(false),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "vote" => {
            let tg = core().tg()?;
            let opt: Option<Vec<u8>> = v
                .get("option")
                .and_then(|x| x.as_str())
                .map(|o| o.as_bytes().to_vec());
            tg.vote(i64f(v, "chat")?, i32f(v, "id")?, opt.unwrap_or_default())
                .await?;
            Ok(json!({}))
        }
        "poll_voters" => {
            let tg = core().tg()?;
            let (count, names) = tg
                .poll_voters(i64f(v, "chat")?, i32f(v, "id")?, None)
                .await?;
            Ok(json!({"count": count, "names": names}))
        }
        "stop_poll" => {
            let tg = core().tg()?;
            tg.stop_poll(i64f(v, "chat")?, i32f(v, "id")?).await?;
            Ok(json!({}))
        }
        // -- реакции --
        "react" => {
            let tg = core().tg()?;
            // react_to(chat, msg, emoji) — через трейт? Проверяем имя ниже.
            tg.react(i64f(v, "chat")?, i32f(v, "id")?, &s(v, "emoji")?)
                .await?;
            Ok(json!({}))
        }
        "reactions" => {
            let tg = core().tg()?;
            Ok(json!(
                tg.message_reactions(i64f(v, "chat")?, i32f(v, "id")?)
                    .await?
            ))
        }
        // -- сторис --
        "stories" => {
            let tg = core().tg()?;
            Ok(json!(tg.stories().await?))
        }
        "story_publish" => {
            let tg = core().tg()?;
            let music = s_opt(v, "music").map(PathBuf::from);
            let id = tg
                .publish_story(
                    &PathBuf::from(s(v, "path")?),
                    &s_opt(v, "caption").unwrap_or_default(),
                    music,
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "forward_story" => {
            let tg = core().tg()?;
            let id = tg
                .forward_story(
                    i64f(v, "peer")?,
                    i32f(v, "story")?,
                    &s_opt(v, "caption").unwrap_or_default(),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "edit_story_caption" => {
            let tg = core().tg()?;
            tg.edit_story_caption(i32f(v, "story")?, &s(v, "caption")?)
                .await?;
            Ok(json!({}))
        }
        "delete_stories" => {
            let tg = core().tg()?;
            let ids: Vec<i32> = v
                .get("ids")
                .and_then(|x| x.as_array())
                .context("нет ids[]")?
                .iter()
                .filter_map(|x| x.as_i64().map(|n| n as i32))
                .collect();
            tg.delete_stories(&ids).await?;
            Ok(json!({}))
        }
        "story_react" => {
            let tg = core().tg()?;
            tg.story_react(i64f(v, "peer")?, i32f(v, "story")?, &s(v, "emoji")?)
                .await?;
            Ok(json!({}))
        }
        "story_reply" => {
            let tg = core().tg()?;
            let id = tg
                .story_reply(i64f(v, "peer")?, i32f(v, "story")?, &s(v, "text")?)
                .await?;
            Ok(json!({"id": id}))
        }
        "story_views" => {
            let tg = core().tg()?;
            Ok(json!({"count": tg.story_views(i32f(v, "story")?).await?}))
        }
        "story_viewers" => {
            let tg = core().tg()?;
            let (count, viewers) = tg.story_viewers(i32f(v, "story")?, limit(v)).await?;
            Ok(json!({"count": count, "viewers": viewers}))
        }
        "toggle_story_pin" => {
            let tg = core().tg()?;
            tg.toggle_story_pin(i32f(v, "story")?, boolf(v, "pinned")?)
                .await?;
            Ok(json!({}))
        }
        "hide_peer_stories" => {
            let tg = core().tg()?;
            tg.hide_peer_stories(i64f(v, "peer")?, boolf(v, "hidden")?)
                .await?;
            Ok(json!({}))
        }
        "archived_stories" => {
            let tg = core().tg()?;
            Ok(json!(tg.archived_stories().await?))
        }
        "story_albums" => {
            let tg = core().tg()?;
            Ok(json!(tg.story_albums().await?))
        }
        "create_album" => {
            let tg = core().tg()?;
            let ids: Vec<i32> = v
                .get("ids")
                .and_then(|x| x.as_array())
                .context("нет ids[]")?
                .iter()
                .filter_map(|x| x.as_i64().map(|n| n as i32))
                .collect();
            let id = tg.create_album(&s(v, "title")?, &ids).await?;
            Ok(json!({"id": id}))
        }
        "album_stories" => {
            let tg = core().tg()?;
            Ok(json!(tg.album_stories(i32f(v, "album")?).await?))
        }
        "enable_stealth" => {
            let tg = core().tg()?;
            tg.enable_stealth().await?;
            Ok(json!({}))
        }
        // -- контакты/участники --
        "contacts" => {
            let tg = core().tg()?;
            Ok(json!(tg.contacts().await?))
        }
        "add_contact" => {
            let tg = core().tg()?;
            tg.add_contact(
                &s(v, "phone")?,
                &s(v, "first")?,
                &s_opt(v, "last").unwrap_or_default(),
            )
            .await?;
            Ok(json!({}))
        }
        "block_user" => {
            let tg = core().tg()?;
            tg.block_user(i64f(v, "chat")?, boolf(v, "blocked")?)
                .await?;
            Ok(json!({}))
        }
        "blocked_list" => {
            let tg = core().tg()?;
            Ok(json!(tg.blocked_list().await?))
        }
        "participants" => {
            let tg = core().tg()?;
            Ok(json!(tg.participants(i64f(v, "chat")?, limit(v)).await?))
        }
        "invite_link" => {
            let tg = core().tg()?;
            Ok(json!({"link": tg.invite_link(i64f(v, "chat")?).await?}))
        }
        "revoke_invite" => {
            let tg = core().tg()?;
            tg.revoke_invite(i64f(v, "chat")?, &s(v, "link")?).await?;
            Ok(json!({}))
        }
        "leave_channel" => {
            let tg = core().tg()?;
            tg.leave_channel(i64f(v, "chat")?).await?;
            Ok(json!({}))
        }
        "set_admin" => {
            let tg = core().tg()?;
            let rights: crate::telegram::AdminRights =
                serde_json::from_value(v.get("rights").cloned().unwrap_or(json!({})))?;
            tg.set_admin(
                i64f(v, "chat")?,
                i64f(v, "user")?,
                i64f(v, "hash")?,
                rights,
                &s_opt(v, "title").unwrap_or_default(),
            )
            .await?;
            Ok(json!({}))
        }
        // -- статистика/инфо --
        "channel_stats" => {
            let tg = core().tg()?;
            Ok(json!(tg.channel_stats(i64f(v, "chat")?).await?))
        }
        "channel_graph" => {
            let tg = core().tg()?;
            let (labels, values) = tg.channel_graph(i64f(v, "chat")?, &s(v, "kind")?).await?;
            Ok(json!({"labels": labels, "values": values}))
        }
        "chat_full" => {
            let tg = core().tg()?;
            Ok(json!(tg.chat_full(i64f(v, "chat")?).await?))
        }
        "peer_full" => {
            let tg = core().tg()?;
            Ok(json!(tg.peer_full(i64f(v, "chat")?).await?))
        }
        // -- поиск/перевод/гео --
        "global_search" => {
            let tg = core().tg()?;
            Ok(json!(tg.global_search(&s(v, "query")?, limit(v)).await?))
        }
        "search_media" => {
            let tg = core().tg()?;
            Ok(json!(
                tg.search_media(i64f(v, "chat")?, &s(v, "kind")?, limit(v))
                    .await?
            ))
        }
        "translate" => {
            let tg = core().tg()?;
            Ok(
                json!({"text": tg.translate_message(i64f(v, "chat")?, i32f(v, "id")?, &s(v, "to")?).await?}),
            )
        }
        "send_geo" => {
            let tg = core().tg()?;
            let lat = v.get("lat").and_then(|x| x.as_f64()).context("нет lat")?;
            let long = v.get("long").and_then(|x| x.as_f64()).context("нет long")?;
            let id = tg.send_geo(i64f(v, "chat")?, lat, long).await?;
            Ok(json!({"id": id}))
        }
        "nearby" => {
            let tg = core().tg()?;
            let lat = v.get("lat").and_then(|x| x.as_f64()).context("нет lat")?;
            let long = v.get("long").and_then(|x| x.as_f64()).context("нет long")?;
            Ok(json!(tg.nearby(lat, long).await?))
        }
        "topic_history" => {
            let tg = core().tg()?;
            Ok(json!(
                tg.topic_history(i64f(v, "chat")?, i32f(v, "topic")?, limit(v))
                    .await?
            ))
        }
        "dialog_filters" => {
            let tg = core().tg()?;
            Ok(json!(tg.dialog_filters().await?))
        }
        // -- эмодзи/профиль --
        "set_emoji_status" => {
            let tg = core().tg()?;
            tg.set_emoji_status(opt_i32(v, "doc").map(|n| n as i64))
                .await?;
            Ok(json!({}))
        }
        "default_emoji_statuses" => {
            let tg = core().tg()?;
            Ok(json!(tg.default_emoji_statuses().await?))
        }
        "update_profile" => {
            let tg = core().tg()?;
            tg.update_profile(
                &s_opt(v, "first").unwrap_or_default(),
                &s_opt(v, "last").unwrap_or_default(),
                &s_opt(v, "about").unwrap_or_default(),
            )
            .await?;
            Ok(json!({}))
        }
        "upload_avatar" => {
            let tg = core().tg()?;
            tg.upload_avatar(&PathBuf::from(s(v, "path")?)).await?;
            Ok(json!({}))
        }
        "check_username" => {
            let tg = core().tg()?;
            Ok(json!({"free": tg.check_username(&s(v, "username")?).await?}))
        }
        "update_username" => {
            let tg = core().tg()?;
            tg.update_username(&s(v, "username")?).await?;
            Ok(json!({}))
        }
        // -- сессии/2fa --
        "sessions" => {
            let tg = core().tg()?;
            Ok(json!(tg.sessions().await?))
        }
        "terminate_session" => {
            let tg = core().tg()?;
            tg.terminate_session(i64f(v, "hash")?).await?;
            Ok(json!({}))
        }
        "terminate_others" => {
            let tg = core().tg()?;
            tg.terminate_others().await?;
            Ok(json!({}))
        }
        "change_password" => {
            let tg = core().tg()?;
            tg.change_password(
                &s(v, "current")?,
                &s(v, "new")?,
                &s_opt(v, "hint").unwrap_or_default(),
                &s_opt(v, "email").unwrap_or_default(),
            )
            .await?;
            Ok(json!({}))
        }
        // -- звёзды/подарки --
        "stars_balance" => {
            let tg = core().tg()?;
            Ok(json!(tg.stars_balance().await?))
        }
        "stars_transactions" => {
            let tg = core().tg()?;
            let inb = v.get("inbound").and_then(|x| x.as_bool()).unwrap_or(true);
            let out = v.get("outbound").and_then(|x| x.as_bool()).unwrap_or(true);
            let lim = v.get("limit").and_then(|x| x.as_i64()).unwrap_or(50) as i32;
            Ok(json!(tg.stars_transactions(inb, out, lim).await?))
        }
        "saved_star_gifts" => {
            let tg = core().tg()?;
            Ok(json!(tg.saved_star_gifts().await?))
        }
        "star_gift_options" => {
            let tg = core().tg()?;
            Ok(json!(tg.star_gift_options(i64f(v, "chat")?).await?))
        }
        "gift_catalog" => {
            let tg = core().tg()?;
            Ok(json!(tg.gift_catalog().await?))
        }
        "send_star_gift" => {
            let tg = core().tg()?;
            let summary = tg
                .send_star_gift(
                    i64f(v, "chat")?,
                    i64f(v, "gift")?,
                    v.get("hide").and_then(|x| x.as_bool()).unwrap_or(false),
                    &s_opt(v, "message").unwrap_or_default(),
                )
                .await?;
            Ok(json!({"summary": summary}))
        }
        // -- медиа/скачивание --
        "download_media" => {
            let tg = core().tg()?;
            let dir = core().dir.join("downloads");
            let path = tg
                .download_message_media(i64f(v, "chat")?, i32f(v, "id")?, &dir)
                .await?;
            Ok(json!({"path": path.to_string_lossy()}))
        }
        "download_story_media" => {
            let tg = core().tg()?;
            let dir = core().dir.join("downloads");
            let path = tg
                .download_story_media(i64f(v, "peer")?, i32f(v, "story")?, &dir)
                .await?;
            Ok(json!({"path": path.to_string_lossy()}))
        }
        "local_duplicates" => {
            // Похожие картинки в локальных папках (pHash + BK-дерево/DSU).
            // Аналог десктопного блока «Похожие» в Загрузках.
            let dirs: Vec<std::path::PathBuf> = v
                .get("dirs")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str())
                        .map(std::path::PathBuf::from)
                        .collect()
                })
                .unwrap_or_default();
            let threshold = v
                .get("threshold")
                .and_then(|x| x.as_u64())
                .unwrap_or(crate::ai::phash::MAX_HAMMING as u64) as u32;
            let max_files = v.get("max_files").and_then(|x| x.as_u64()).unwrap_or(300) as usize;
            let mut paths = Vec::new();
            for d in &dirs {
                collect_image_files(d, &mut paths, max_files.saturating_mul(3));
            }
            paths.sort();
            paths.dedup();
            paths.truncate(max_files);
            let mut hashes = Vec::new();
            let mut ok_paths = Vec::new();
            for p in paths {
                if let Ok(img) = image::open(&p) {
                    hashes.push(crate::ai::phash::phash(&img));
                    ok_paths.push(p);
                }
            }
            let groups = crate::ai::cluster::cluster_indices(&hashes, threshold.min(64));
            let grouped: Vec<Value> = groups
                .iter()
                .map(|g| {
                    json!(g
                        .iter()
                        .filter_map(|i| ok_paths.get(*i))
                        .map(|p| p.to_string_lossy().to_string())
                        .collect::<Vec<_>>())
                })
                .collect();
            Ok(json!({"groups": grouped, "scanned": ok_paths.len()}))
        }
        // -- экспорт (без диалога: путь задаёт Kotlin) --
        "export_history" => {
            let tg = core().tg()?;
            let chat = i64f(v, "chat")?;
            let format = s_opt(v, "format").unwrap_or_else(|| "json".into());
            let with_media = v
                .get("with_media")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let path = PathBuf::from(s(v, "path")?);
            let mut all = vec![];
            let mut cursor = i32::MAX;
            for _ in 0..40 {
                let page = tg.history_before(chat, cursor, 500).await?;
                if page.is_empty() {
                    break;
                }
                cursor = page.last().map(|m| m.id).unwrap_or(0);
                all.extend(page);
                if all.len() >= 20_000 {
                    break;
                }
            }
            all.sort_by_key(|m| m.id);
            let mut media: std::collections::HashMap<i32, String> = Default::default();
            if with_media && format == "html" {
                let dir = path.with_extension("files");
                std::fs::create_dir_all(&dir).ok();
                let mut done = 0;
                for m in all.iter().filter(|m| m.has_media) {
                    if done >= 150 {
                        break;
                    }
                    if let Ok(p) = tg.download_message_media(chat, m.id, &dir).await {
                        if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                            media.insert(
                                m.id,
                                format!(
                                    "{}/{}",
                                    dir.file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default(),
                                    name
                                ),
                            );
                            done += 1;
                        }
                    }
                }
            }
            let text = if format == "html" {
                render_export_html("chat", &all, &media)
            } else {
                serde_json::to_string_pretty(&all)?
            };
            std::fs::write(&path, text)?;
            Ok(json!({"path": path.to_string_lossy(), "count": all.len()}))
        }
        // -- анти-реклама сторис --
        "ad_score" => {
            use crate::ai::AdDetector;
            let det = crate::ai::HeuristicAdDetector::new();
            let score = det.score_story(
                &s_opt(v, "caption").unwrap_or_default(),
                v.get("has_url").and_then(|x| x.as_bool()).unwrap_or(false),
                v.get("has_qr").and_then(|x| x.as_bool()).unwrap_or(false),
            );
            Ok(json!(score))
        }
        "qr_hint" => {
            // Есть ли QR-код на картинке (эвристика для сторис).
            let img = image::open(PathBuf::from(s(v, "path")?))?;
            Ok(json!({"has_qr": crate::ai::blur::looks_like_qr(&img)}))
        }
        "blur_full" => {
            let sigma = v.get("sigma").and_then(|x| x.as_f64()).unwrap_or(8.0) as f32;
            let img = image::open(PathBuf::from(s(v, "src")?))?;
            let out = crate::ai::blur::blur_full(&img, sigma);
            let dst = PathBuf::from(s(v, "dst")?);
            out.save(&dst)?;
            Ok(json!({"path": dst.to_string_lossy()}))
        }
        // -- яндекс --
        "yandex_upload" => {
            crate::integrations::yandex::ensure_folder(&s(v, "token")?, &s(v, "remote_folder")?)
                .await?;
            crate::integrations::yandex::upload_file(
                &s(v, "token")?,
                &PathBuf::from(s(v, "local")?),
                &s(v, "remote")?,
            )
            .await?;
            Ok(json!({}))
        }
        // -- qr в картинку --
        "qr_bitmap" => {
            let png = qr_payload_to_png_232(&s(v, "payload")?)?;
            Ok(json!({"png_b64": b64(&png)}))
        }
        // -- голос --
        "wav_to_ogg" => {
            let dst = PathBuf::from(s(v, "dst")?);
            crate::media::voice_rec::wav_file_to_ogg_voice(&PathBuf::from(s(v, "src")?), &dst)?;
            Ok(json!({"path": dst.to_string_lossy()}))
        }
        "recent_stickers" => {
            let tg = core().tg()?;
            Ok(json!(tg.recent_stickers().await?))
        }
        "saved_gifs" => {
            let tg = core().tg()?;
            Ok(json!(tg.saved_gifs().await?))
        }
        "avatar_path" => {
            let tg = core().tg()?;
            Ok(
                json!({"path": tg.avatar_path(i64f(v, "chat")?).await?.map(|p| p.to_string_lossy().to_string())}),
            )
        }
        "create_channel" => {
            let tg = core().tg()?;
            let id = tg
                .create_channel(
                    &s(v, "title")?,
                    &s_opt(v, "about").unwrap_or_default(),
                    v.get("broadcast")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "create_topic" => {
            let tg = core().tg()?;
            let id = tg.create_topic(i64f(v, "chat")?, &s(v, "title")?).await?;
            Ok(json!({"id": id}))
        }
        "forum_topics" => {
            let tg = core().tg()?;
            Ok(json!(tg.forum_topics(i64f(v, "chat")?).await?))
        }
        "send_to_topic" => {
            let tg = core().tg()?;
            let id = tg
                .send_to_topic(
                    i64f(v, "chat")?,
                    i32f(v, "topic")?,
                    &s(v, "text")?,
                    v.get("silent").and_then(|x| x.as_bool()).unwrap_or(false),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "delete_album" => {
            let tg = core().tg()?;
            tg.delete_album(i32f(v, "album")?).await?;
            Ok(json!({}))
        }
        "delete_contact" => {
            let tg = core().tg()?;
            tg.delete_contact(i64f(v, "user")?, i64f(v, "hash")?)
                .await?;
            Ok(json!({}))
        }
        "download_web_photo" => {
            let tg = core().tg()?;
            let photo: crate::telegram::WebPreviewPhoto =
                serde_json::from_value(v.get("photo").cloned().context("нет photo")?)?;
            let dir = core().dir.join("downloads");
            let path = tg.download_web_photo(&photo, &dir).await?;
            Ok(json!({"path": path.to_string_lossy()}))
        }
        "get_profile" => {
            let tg = core().tg()?;
            Ok(json!(tg.get_profile().await?))
        }
        "kick_participant" => {
            let tg = core().tg()?;
            tg.kick_participant(i64f(v, "chat")?, i64f(v, "user")?)
                .await?;
            Ok(json!({}))
        }
        "share_contact" => {
            let tg = core().tg()?;
            let id = tg
                .share_contact(
                    i64f(v, "chat")?,
                    &s(v, "phone")?,
                    &s(v, "first")?,
                    &s_opt(v, "last").unwrap_or_default(),
                )
                .await?;
            Ok(json!({"id": id}))
        }
        "pin_message" => {
            let tg = core().tg()?;
            tg.pin_message(i64f(v, "chat")?, i32f(v, "id")?).await?;
            Ok(json!({}))
        }
        "unpin_message" => {
            let tg = core().tg()?;
            tg.unpin_message(i64f(v, "chat")?, i32f(v, "id")?).await?;
            Ok(json!({}))
        }
        "story_viewers" => {
            let tg = core().tg()?;
            let (count, viewers) = tg.story_viewers(i32f(v, "story")?, limit(v)).await?;
            Ok(json!({"count": count, "viewers": viewers}))
        }
        "test_proxy" => {
            // Проверка SOCKS5 отдельным соединением (сессия-проба в tmp, drop после).
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let probe =
                std::env::temp_dir().join(format!("telecomfort_proxy_probe_{nanos}.session"));
            let api_id: i32 = v.get("api_id").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let proxy = s_opt(v, "proxy").filter(|p| !p.is_empty());
            let backend = crate::telegram::backend_grammers::GrammersBackend::connect_with_proxy(
                &probe,
                api_id,
                &s(v, "api_hash")?,
                proxy,
            )
            .await?;
            let authed = backend.is_authorized().await.unwrap_or(false);
            let _ = std::fs::remove_file(&probe);
            Ok(json!({"ok_proxy": true, "authorized": authed}))
        }
        "poll_updates" => {
            // Long-poll live-обновлений (без FCM): новое сообщение/правка/звонок.
            let tg = core().tg()?;
            let ms = v
                .get("timeout_ms")
                .and_then(|x| x.as_u64())
                .unwrap_or(25_000)
                .min(60_000);
            Ok(json!({"update": tg.next_live_update(ms).await?}))
        }
        "decline_call" => {
            let tg = core().tg()?;
            tg.decline_call(i64f(v, "call_id")?, i64f(v, "access_hash")?)
                .await?;
            Ok(json!({}))
        }
        "is_authorized" => {
            let tg = core().tg()?;
            // is_authorized — inherent-метод бэкенда: пробуем через downcast.
            // Универсально: me() удался => авторизован.
            match tg.me().await {
                Ok(_) => Ok(json!({"authorized": true})),
                Err(_) => Ok(json!({"authorized": false})),
            }
        }
        "logout" => {
            // Выход: забыть сервис и стереть session-файл (как десктопный logout).
            let mut c = core();
            c.tg = None;
            let sess = c.dir.join("session_android.session");
            let _ = std::fs::remove_file(&sess);
            // SQLite-journal рядом с сессией (grammers/rusqlite).
            let mut journal = sess.as_os_str().to_owned();
            journal.push("-journal");
            let _ = std::fs::remove_file(std::path::PathBuf::from(journal));
            Ok(json!({"ok": true}))
        }
        _ => anyhow::bail!("unknown op: {op}"),
    }
}

// --- чистые хелперы (без сети, тестируются на хосте) ------------------------

/// QR 232px → PNG-байты (для показа кода в Kotlin Image).
pub fn qr_payload_to_png_232(payload: &str) -> Result<Vec<u8>> {
    use image::GenericImageView;
    let code = qrcode::QrCode::new(payload.as_bytes()).context("qr encode")?;
    let w = code.width() as u32;
    let scale = 8u32;
    let border = 4u32;
    let side = (w + border * 2) * scale;
    let mut img = image::GrayImage::new(side, side);
    for y in 0..side {
        for x in 0..side {
            img.put_pixel(x, y, image::Luma([255u8]));
        }
    }
    for y in 0..w {
        for x in 0..w {
            let dark = code[(x as usize, y as usize)] == qrcode::Color::Dark;
            let v = if dark { 0u8 } else { 255u8 };
            for dy in 0..scale {
                for dx in 0..scale {
                    img.put_pixel(
                        (x + border) * scale + dx,
                        (y + border) * scale + dy,
                        image::Luma([v]),
                    );
                }
            }
        }
    }
    // Центр-кроп/паддинг до 232.
    const TARGET: u32 = 232;
    let mut out = image::GrayImage::new(TARGET, TARGET);
    for y in 0..TARGET {
        for x in 0..TARGET {
            out.put_pixel(x, y, image::Luma([255u8]));
        }
    }
    let copy = side.min(TARGET);
    let (sx, sy) = (
        (side.saturating_sub(TARGET)) / 2,
        (side.saturating_sub(TARGET)) / 2,
    );
    let (dx, dy) = (
        (TARGET.saturating_sub(side)) / 2,
        (TARGET.saturating_sub(side)) / 2,
    );
    for y in 0..copy {
        for x in 0..copy {
            out.put_pixel(dx + x, dy + y, *img.get_pixel(sx + x, sy + y));
        }
    }
    let mut buf = vec![];
    let enc = image::codecs::png::PngEncoder::new(&mut buf);
    use image::ImageEncoder;
    enc.write_image(out.as_raw(), TARGET, TARGET, image::ExtendedColorType::L8)?;
    Ok(buf)
}

/// HTML-экспорт для моста (зеркало ui::app::export_history_html — ui нет на Android).
pub fn render_export_html(
    title: &str,
    items: &[crate::telegram::MessageInfo],
    media: &std::collections::HashMap<i32, String>,
) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    let mut rows = String::new();
    for m in items {
        let sender = m.sender.clone().unwrap_or_else(|| {
            if m.outgoing {
                "я".into()
            } else {
                "—".into()
            }
        });
        let mut body = esc(&m.text).replace('\n', "<br>");
        if m.has_media {
            let kind = m.media_kind.clone().unwrap_or_else(|| "медиа".into());
            let name = m.file_name.clone().unwrap_or_default();
            if let Some(local) = media.get(&m.id) {
                body += &format!(
                    "<br><a href=\"{}\">[{} {}]</a>",
                    esc(local),
                    esc(&kind),
                    esc(&name)
                );
            } else {
                body += &format!("<br><i>[{} {}]</i>", esc(&kind), esc(&name));
            }
        }
        rows += &format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            m.date,
            esc(&sender),
            body
        );
    }
    format!(
        "<!DOCTYPE html><html lang=\"ru\"><head><meta charset=\"utf-8\"><title>{}</title></head>\
        <body><h1>{}</h1><p>Сообщений: {}</p>\
        <table border=\"1\" cellpadding=\"4\"><tr><th>Unix</th><th>Автор</th><th>Текст</th></tr>{}</table></body></html>",
        esc(title), esc(title), items.len(), rows
    )
}

// --- JNI -------------------------------------------------------------------
/// JNI-точка входа: `Core.nativeCall(cmd): String` (см. android/app).
///
/// Сигнатура и обработка `EnvOutcome` — по канону jni 0.22: `EnvUnowned` →
/// `with_env` (catch_unwind) → `resolve` с политикой (null при ошибке).
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_telecomfort_core_Core_nativeCall<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    cmd: jni::objects::JString<'caller>,
) -> jni::sys::jstring {
    unowned_env
        .with_env(|env| -> jni::errors::Result<jni::sys::jstring> {
            let req: String = cmd.try_to_string(env)?;
            let resp = core_call(&req);
            let out = jni::objects::JString::from_str(env, &resp)?;
            Ok(out.into_raw())
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(op: serde_json::Value) -> Value {
        let s = core_call(&serde_json::to_string(&op).unwrap());
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn bridge_uses_mock_and_lists_dialogs() {
        let r = call(json!({"op": "use_mock"}));
        assert_eq!(r["ok"], true);
        let r = call(json!({"op": "dialogs"}));
        assert_eq!(r["ok"], true);
        assert!(r["result"].is_array());
        let r = call(
            json!({"op": "stars_transactions", "inbound": true, "outbound": true, "limit": 5}),
        );
        assert_eq!(r["ok"], true);
        assert!(r["result"].is_array());
    }

    #[test]
    fn bridge_auth_flow_mock() {
        call(json!({"op": "use_mock"}));
        let r = call(json!({"op": "request_code", "phone": "+7000"}));
        assert_eq!(r["ok"], true);
        let key = r["key"].as_str().unwrap().to_string();
        let r = call(json!({"op": "sign_in_code", "key": key, "code": "00000"}));
        assert_eq!(r["status"], "ok");
        let r = call(json!({"op": "password_hint"}));
        assert_eq!(r["ok"], true);
    }

    #[test]
    fn bridge_password_required_path() {
        call(json!({"op": "use_mock"}));
        let r = call(json!({"op": "sign_in_code", "key": "mock-login-key", "code": "11111"}));
        assert_eq!(r["status"], "password_required");
        let r =
            call(json!({"op": "sign_in_password", "key": "mock-login-key", "password": "demo"}));
        assert_eq!(r["status"], "ok");
    }

    #[test]
    fn bridge_new_ops_smoke() {
        call(json!({"op": "use_mock"}));
        for op in [
            json!({"op": "get_profile"}),
            json!({"op": "forum_topics", "chat": -1001}),
            json!({"op": "create_topic", "chat": -1001, "title": "t"}),
            json!({"op": "create_channel", "title": "c", "about": "", "broadcast": true}),
            json!({"op": "delete_album", "album": 1}),
            json!({"op": "delete_contact", "user": 1, "hash": 0}),
            json!({"op": "get_profile"}),
            json!({"op": "kick_participant", "chat": -1001, "user": 1}),
            json!({"op": "share_contact", "chat": -1001, "phone": "+7", "first": "A"}),
            json!({"op": "pin_message", "chat": -1001, "id": 1}),
            json!({"op": "unpin_message", "chat": -1001, "id": 1}),
            json!({"op": "send_to_topic", "chat": -1001, "topic": 1, "text": "hi"}),
        ] {
            let r = call(op);
            assert_eq!(r["ok"], true, "{r}");
        }
    }

    #[test]
    fn bridge_round3_ops_smoke() {
        call(json!({"op": "use_mock"}));
        for op in [
            // участники/инфо/планировщик/закреп/опросы
            json!({"op": "participants", "chat": -1001, "limit": 10}),
            json!({"op": "chat_full", "chat": -1001}),
            json!({"op": "scheduled_list", "chat": -1001}),
            json!({"op": "pinned_list", "chat": -1001, "limit": 10}),
            json!({"op": "pinned_one", "chat": -1001}),
            json!({"op": "create_poll", "chat": -1001, "question": "q?", "options": ["a", "b"], "quiz": false}),
            json!({"op": "vote", "chat": -1001, "id": 1}),
            json!({"op": "poll_voters", "chat": -1001, "id": 1}),
            json!({"op": "stop_poll", "chat": -1001, "id": 1}),
            // папки/черновики/перевод/гео
            json!({"op": "dialog_filters"}),
            json!({"op": "create_folder", "title": "f", "peers": []}),
            json!({"op": "rename_folder", "id": 1, "title": "f2"}),
            json!({"op": "delete_folder", "id": 1}),
            json!({"op": "save_draft", "chat": -1001, "text": "draft"}),
            json!({"op": "send_to_topic", "chat": -1001, "topic": 1, "text": "t"}),
            json!({"op": "topic_history", "chat": -1001, "topic": 1, "limit": 5}),
            json!({"op": "send_geo", "chat": -1001, "lat": 1.0, "long": 2.0}),
            json!({"op": "nearby", "lat": 1.0, "long": 2.0}),
            // медиа-загрузки/сторис-сервис
            json!({"op": "download_media", "chat": -1001, "id": 1}),
            json!({"op": "download_story_media", "peer": 1, "story": 1}),
            json!({"op": "toggle_story_pin", "story": 1, "pinned": true}),
            json!({"op": "hide_peer_stories", "peer": 1, "hidden": true}),
            json!({"op": "archived_stories"}),
            json!({"op": "story_albums"}),
            json!({"op": "create_album", "title": "a", "ids": [1]}),
            json!({"op": "album_stories", "album": 1}),
            json!({"op": "delete_album", "album": 1}),
            json!({"op": "enable_stealth"}),
            json!({"op": "story_react", "peer": 1, "story": 1, "emoji": "❤"}),
            json!({"op": "story_reply", "peer": 1, "story": 1, "text": "hi"}),
            json!({"op": "story_views", "story": 1}),
            json!({"op": "story_viewers", "story": 1, "limit": 10}),
            json!({"op": "forward_story", "peer": 1, "story": 1}),
            json!({"op": "edit_story_caption", "story": 1, "caption": "c"}),
            json!({"op": "delete_stories", "ids": [1]}),
            // export with media via mock download
            json!({"op": "export_history", "chat": -1001, "format": "html", "with_media": true,
                   "path": std::env::temp_dir().join("bridge_export_test.html").to_string_lossy()}),
        ] {
            let r = call(op);
            assert_eq!(r["ok"], true, "{r}");
        }
    }

    #[test]
    fn bridge_round4_ops() {
        call(json!({"op": "use_mock"}));
        for op in [
            json!({"op": "download_web_photo", "photo": {"photo_id": 1, "access_hash": 2, "file_reference": [], "dc_id": 2}, "chat": 1}),
            json!({"op": "send_to_topic", "chat": -1001, "topic": 1, "text": "t"}),
            json!({"op": "topic_history", "chat": -1001, "topic": 1, "limit": 5}),
            json!({"op": "unpin_message", "chat": -1001, "id": 1}),
            json!({"op": "forum_topics", "chat": -1001}),
            json!({"op": "create_topic", "chat": -1001, "title": "t"}),
            json!({"op": "create_channel", "title": "c", "about": "", "broadcast": false}),
            json!({"op": "delete_album", "album": 1}),
            json!({"op": "kick_participant", "chat": -1001, "user": 1}),
            json!({"op": "share_contact", "chat": -1001, "phone": "+7", "first": "A"}),
            json!({"op": "enable_stealth"}),
            json!({"op": "create_poll", "chat": -1001, "question": "q?", "options": ["a"], "quiz": true, "correct": 0}),
            json!({"op": "revoke_invite", "chat": -1001, "link": "https://t.me/x"}),
            json!({"op": "rename_folder", "id": 1, "title": "renamed"}),
            json!({"op": "hide_peer_stories", "peer": 1, "hidden": true}),
            json!({"op": "toggle_story_pin", "story": 1, "pinned": true}),
            json!({"op": "vote", "chat": -1001, "id": 1, "option": "AA=="}),
            json!({"op": "stop_poll", "chat": -1001, "id": 1}),
            json!({"op": "poll_voters", "chat": -1001, "id": 1}),
            json!({"op": "qr_bitmap", "payload": "test"}),
            json!({"op": "archived_stories"}),
            json!({"op": "avatar_path", "chat": -1001}),
            json!({"op": "pinned_one", "chat": -1001}),
            json!({"op": "is_authorized"}),
            json!({"op": "poll_updates", "timeout_ms": 1}),
            json!({"op": "decline_call", "call_id": 1, "access_hash": 2}),
        ] {
            let r = call(op);
            assert_eq!(r["ok"], true, "{r}");
        }
    }

    /// Ручной тест прокси: требует сеть, поэтому ignored (запуск: -- --ignored).
    /// Проверяет, что connect_with_proxy не паникует и возвращает JSON-ошибку,
    /// а не роняет процесс (libsql threading в мультитред-рантайме).
    #[test]
    #[ignore]
    fn bridge_test_proxy_manual() {
        let r = call(json!({"op": "test_proxy", "api_id": 0, "api_hash": "x"}));
        assert!(r.get("ok").is_some(), "{r}");
    }

    #[test]
    fn bridge_local_duplicates_finds_groups() {
        let dir = std::env::temp_dir().join(format!("tc_dup_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = image::RgbImage::from_fn(64, 64, |x, y| {
            image::Rgb([(x * 4) as u8, (y * 4) as u8, 128])
        });
        img.save(dir.join("a.png")).unwrap();
        let dynimg = image::DynamicImage::ImageRgb8(img);
        let small = image::imageops::resize(&dynimg, 48, 48, image::imageops::FilterType::Triangle);
        small.save(dir.join("b.png")).unwrap();
        let r = call(
            json!({"op": "local_duplicates", "dirs": [dir.to_string_lossy()], "threshold": 6}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let groups = r["groups"].as_array().cloned().unwrap_or_default();
        assert!(!groups.is_empty(), "{r}");
        assert!(
            groups[0].as_array().map(|g| g.len() >= 2).unwrap_or(false),
            "{r}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bridge_unknown_op_errors() {
        let r = call(json!({"op": "nope"}));
        assert_eq!(r["ok"], false);
    }

    #[test]
    fn bridge_ad_score_and_qr_png() {
        let r = call(
            json!({"op": "ad_score", "caption": "СКИДКА 50% жми ссылку", "has_url": true, "has_qr": false}),
        );
        assert_eq!(r["ok"], true);
        assert!(r["score"].as_f64().unwrap_or(0.0) > 0.3);
        let r = call(json!({"op": "qr_bitmap", "payload": "tg://login?token=QUJD"}));
        assert_eq!(r["ok"], true);
        let raw = b64dec(r["png_b64"].as_str().unwrap()).unwrap();
        assert!(raw.starts_with(&[137, 80, 78, 71])); // PNG magic
    }

    #[test]
    fn bridge_export_json_mock() {
        call(json!({"op": "use_mock"}));
        let dir = std::env::temp_dir().join("telecomfort_bridge_test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("h.json");
        let r = call(
            json!({"op": "export_history", "chat": -1001, "format": "json", "path": path.to_string_lossy()}),
        );
        assert_eq!(r["ok"], true);
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bridge_export_with_media_html() {
        call(json!({"op": "use_mock"}));
        let dir = std::env::temp_dir().join("telecomfort_bridge_export");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("h.html");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(dir.join("h.files"));
        let r = call(
            json!({"op": "export_history", "chat": -1001, "format": "html",
            "with_media": true, "path": path.to_string_lossy()}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("<table"), "{text:.200}");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(dir.join("h.files"));
    }
}
