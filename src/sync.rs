//! Фоновая синхронизация: диалоги → сторис → новые медиа → автозагрузка.
//!
//! Запускается из `TelecomfortApp::start_background_sync` как tokio-задача.
//! Каждые `interval_secs`:
//!  1. обновляет диалоги и сторис (через те же BgEvent-ы, что и ручные кнопки),
//!  2. для каждого чата смотрит `history(limit=30)`, сравнивает max message_id
//!     с `sync_state` в SQLite и докачивает только новое по правилам,
//!  3. шлёт десктоп-уведомления (notify-rust) о загрузках/рекламе.

use crate::config::AppConfig;
use crate::db::Db;
use crate::media::AutoDownloader;
use crate::telegram::TelegramService;
use crate::ui::app::BgEvent;
use std::sync::{mpsc, Arc};

pub fn notify(title: &str, body: &str) {
    // Best-effort: в headless/CI просто пишем в лог.
    #[cfg(target_os = "linux")]
    {
        // notify-rust на Linux требует D-Bus; если нет — молча пропускаем.
    }
    match notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .appname("Telecomfort")
        .icon("telecomfort")
        .show()
    {
        Ok(_) => {}
        Err(e) => tracing::debug!("notify skipped: {e}"),
    }
}

pub struct SyncEngine {
    pub cfg_snapshot: AppConfig,
    pub tg: Arc<dyn TelegramService>,
    pub db: Arc<Db>,
    pub tx: mpsc::Sender<BgEvent>,
}

impl SyncEngine {
    /// Одна итерация синка. Возвращает (новых_сториc, новых_загрузок).
    pub async fn run_once(&self) -> (usize, usize) {
        let mut new_stories = 0usize;
        let mut new_downloads = 0usize;

        // --- Сторис ---
        match self.tg.stories().await {
            Ok(stories) => {
                new_stories = stories.len();
                let _ = self.tx.send(BgEvent::Stories(stories));
            }
            Err(e) => {
                let _ = self
                    .tx
                    .send(BgEvent::Log(format!("фон: сторис не обновлены: {e:#}")));
            }
        }

        // --- Диалоги ---
        let dialogs = match self.tg.dialogs().await {
            Ok(d) => {
                let _ = self.tx.send(BgEvent::Dialogs(d.clone()));
                d
            }
            Err(e) => {
                let _ = self
                    .tx
                    .send(BgEvent::Log(format!("фон: диалоги не обновлены: {e:#}")));
                vec![]
            }
        };

        if !self.cfg_snapshot.sync.auto_download_new {
            return (new_stories, 0);
        }

        let downloader = AutoDownloader::new(
            self.cfg_snapshot.downloader.clone(),
            self.cfg_snapshot.yandex.oauth_token.clone(),
            self.cfg_snapshot.yandex.remote_folder.clone(),
            self.cfg_snapshot.yandex.auto_upload,
            self.cfg_snapshot.calibre.library_path.clone(),
            self.cfg_snapshot.calibre.auto_import_books,
            self.cfg_snapshot.calibre.delete_after_import,
            Arc::clone(&self.db),
        )
        .with_progress(self.tx.clone());

        // Собираем задания по всем чатам, затем качаем параллельно
        // (до 3 файлов одновременно): файлы лежат на разных DC Telegram,
        // multi-DC пул разруливает migrate/auth на каждый (см. mtproto.rs).
        let mut jobs: Vec<crate::media::DownloadJob> = Vec::new();
        for chat in dialogs.iter().take(50) {
            let history = match self.tg.history(chat.id, 30).await {
                Ok(h) => h,
                Err(_) => continue,
            };
            if history.is_empty() {
                continue;
            }
            let max_id = history.iter().map(|m| m.id).max().unwrap_or(0);
            let known = self.db.get_last_message_id(chat.id);
            // Первый запуск: запоминаем watermark, но не качаем всю историю.
            if known == 0 {
                self.db.set_last_message_id(chat.id, max_id);
                continue;
            }
            if max_id <= known {
                continue;
            }
            let fresh: Vec<_> = history.into_iter().filter(|m| m.id > known).collect();
            // Обновляем watermark сразу, чтобы не качать дважды.
            self.db.set_last_message_id(chat.id, max_id);
            for m in fresh {
                if !m.has_media {
                    continue;
                }
                let kind =
                    AutoDownloader::classify(m.file_name.as_deref(), m.media_kind.as_deref());
                let size = m.file_size.unwrap_or(0).max(0) as u64;
                if !downloader.should_download(&chat.title, chat.id, &kind, size) {
                    continue;
                }
                jobs.push(crate::media::DownloadJob {
                    chat_id: chat.id,
                    chat_label: chat.title.clone(),
                    message_id: m.id,
                    file_name: m.file_name.clone(),
                    media_kind: m.media_kind.clone(),
                    file_size: m.file_size,
                });
            }
        }
        if jobs.is_empty() {
            return (new_stories, 0);
        }
        for (job, res) in
            crate::media::download_many(&downloader, Arc::clone(&self.tg), jobs, 3).await
        {
            match res {
                Ok(rep) => {
                    new_downloads += 1;
                    let mut s = format!(
                        "фон: {} из {} → {}",
                        rep.local_path
                            .file_name()
                            .map(|x| x.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        job.chat_label,
                        rep.local_path.display()
                    );
                    if !rep.near_duplicate_of.is_empty() {
                        s.push_str(&format!(" (≈ {})", rep.near_duplicate_of));
                    }
                    if rep.calibre_imported {
                        s.push_str(" → Calibre ✓");
                    }
                    let _ = self.tx.send(BgEvent::Log(s.clone()));
                    if self.cfg_snapshot.sync.notify_on_download {
                        notify(
                            "Telecomfort: новый файл",
                            &format!(
                                "{} · {}",
                                job.chat_label,
                                rep.local_path
                                    .file_name()
                                    .map(|x| x.to_string_lossy().to_string())
                                    .unwrap_or_default()
                            ),
                        );
                    }
                }
                Err(e) => {
                    let _ = self.tx.send(BgEvent::Log(format!(
                        "фон: пропуск msg {} из {}: {e:#}",
                        job.message_id, job.chat_label
                    )));
                }
            }
        }

        (new_stories, new_downloads)
    }
}
