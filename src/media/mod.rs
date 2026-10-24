//! Автозагрузчик: правила, обход `noforwards`, связка с Яндекс Диском и Calibre.

pub mod av1;
pub mod h264;
pub mod hevc;
pub mod mtproto;
pub mod video;
pub mod voice_rec;

use crate::config::{DownloaderSettings, MediaKind};
use crate::db::Db;
use crate::integrations::{calibre, yandex};
use crate::telegram::TelegramService;
use crate::ui::app::BgEvent;
use crate::utils;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Лимит параллельных заливок на Яндекс Диск (стриминг больших видео
/// иначе забивает сеть/память). 4 — баланс скорость/стабильность.
pub fn upload_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(4))
}

#[derive(Clone)]
pub struct AutoDownloader {
    settings: DownloaderSettings,
    yandex_token: String,
    yandex_folder: String,
    yandex_auto: bool,
    calibre_lib: String,
    calibre_auto: bool,
    calibre_delete: bool,
    db: Arc<Db>,
    progress_tx: Option<std::sync::mpsc::Sender<BgEvent>>,
    workers: usize,
}

impl AutoDownloader {
    pub fn new(
        settings: DownloaderSettings,
        yandex_token: String,
        yandex_folder: String,
        yandex_auto: bool,
        calibre_lib: String,
        calibre_auto: bool,
        calibre_delete: bool,
        db: Arc<Db>,
    ) -> Self {
        Self {
            settings,
            yandex_token,
            yandex_folder,
            yandex_auto,
            calibre_lib,
            calibre_auto,
            calibre_delete,
            db,
            progress_tx: None,
            workers: mtproto::DEFAULT_WORKERS,
        }
    }

    /// Подключить прогресс заливок в UI (BgEvent::UploadProgress).
    pub fn with_progress(mut self, tx: std::sync::mpsc::Sender<BgEvent>) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    /// Параллельных чанков MTProto на файл (1..=8).
    pub fn with_workers(mut self, n: usize) -> Self {
        self.workers = n.clamp(1, 8);
        self
    }

    pub fn rule_for(&self, chat: &str, kind: &MediaKind, size: u64) -> bool {
        Self::matches_rules(&self.settings, chat, kind, size)
    }

    /// Проверка без создания загрузчика — для фонового синка и UI-подсказок.
    pub fn matches_rules(
        settings: &DownloaderSettings,
        chat: &str,
        kind: &MediaKind,
        size: u64,
    ) -> bool {
        // Пустые kinds = Any (защита от «молча» неработающих правил из UI).
        for r in &settings.rules {
            if !r.enabled {
                continue;
            }
            if r.chat != "*" && r.chat != chat {
                // Дополнительно: матчим по username без @ и по ID строкой
                continue;
            }
            let kinds_empty = r.kinds.is_empty();
            let kind_ok = kinds_empty || r.kinds.iter().any(|k| *k == MediaKind::Any || k == kind);
            if !kind_ok {
                continue;
            }
            if r.max_size_mb > 0 && size > r.max_size_mb * 1024 * 1024 {
                continue;
            }
            return true;
        }
        false
    }

    pub fn should_download(
        &self,
        chat_label: &str,
        chat_id: i64,
        kind: &MediaKind,
        size: u64,
    ) -> bool {
        self.rule_for(chat_label, kind, size) || self.rule_for(&chat_id.to_string(), kind, size)
    }

    pub fn classify(file_name: Option<&str>, media_kind: Option<&str>) -> MediaKind {
        let name = file_name.unwrap_or("").to_lowercase();
        let ext = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let is_video = matches!(media_kind, Some("video"))
            || matches!(ext, "mp4" | "mkv" | "avi" | "mov" | "webm");
        let is_audio = matches!(media_kind, Some("audio"))
            || matches!(ext, "mp3" | "ogg" | "oga" | "opus" | "m4a" | "flac" | "wav");
        let is_photo =
            matches!(media_kind, Some("photo")) || matches!(ext, "jpg" | "jpeg" | "png" | "webp");
        if is_video {
            MediaKind::Video
        } else if is_audio {
            MediaKind::Audio
        } else if is_photo {
            MediaKind::Photo
        } else {
            MediaKind::Document
        }
    }

    /// Скачать одно сообщение (с обходом защиты), затем прогнать по цепочке:
    /// хэш-дедуп → Яндекс Диск → Calibre.
    pub async fn download_one(
        &self,
        tg: &dyn TelegramService,
        chat_id: i64,
        chat_label: &str,
        message_id: i32,
        file_name: Option<String>,
        media_kind: Option<String>,
        file_size: Option<i64>,
    ) -> anyhow::Result<DownloadReport> {
        let kind = Self::classify(file_name.as_deref(), media_kind.as_deref());
        let size = file_size.unwrap_or(0).max(0) as u64;
        if !self.should_download(chat_label, chat_id, &kind, size) {
            anyhow::bail!("не подходит под правила автозагрузки");
        }

        let dest_dir = PathBuf::from(&self.settings.download_dir);
        // ВАЖНО: качаем напрямую через MTProto, игнорируя клиентский флаг
        // noforwards (копирование/пересылка запрещены только в UI).
        // Параллельные чанки + resume + прогресс в UI.
        let label = format!("{chat_label} · msg {message_id}");
        let progress_tx = self.progress_tx.clone();
        let label_cb = label.clone();
        let progress_cb = move |done: u64, total: u64| {
            if let Some(tx) = &progress_tx {
                let _ = tx.send(BgEvent::MediaProgress {
                    label: label_cb.clone(),
                    done,
                    total,
                });
            }
        };
        let path = tg
            .download_message_media_with_progress(
                chat_id,
                message_id,
                &dest_dir,
                self.workers,
                &progress_cb,
            )
            .await?;

        // Дедуп по sha256 (бит-в-бит) + визуальные дубли по pHash.
        let sha = utils::sha256_of_file(&path).unwrap_or_default();
        let duplicate = !sha.is_empty() && self.db.is_duplicate_hash(&sha);
        if !sha.is_empty() {
            self.db.remember_hash(&sha, &path.to_string_lossy());
        }
        // pHash: картинки — напрямую; видео — по первому кадру встроенных
        // декодеров (H.264/HEVC/AV1/covr, без ffmpeg), чтобы видеомемы тоже
        // кластеризовались. Декод в spawn_blocking + таймаут 60с, чтобы
        // тяжёлый файл не вешал фоновую задачу; постер сохраняем рядом
        // с видео — он же превью для групп дублей в Загрузках.
        // Документы/аудио пропускаем (нет видеоряда для хэширования).
        let image_phash = image::open(&path)
            .ok()
            .map(|img| crate::ai::phash::phash(&img));
        let phash: Option<u64> = match image_phash {
            Some(h) => Some(h),
            None if crate::media::video::is_video_path(&path) => {
                let p = path.clone();
                let frame = tokio::time::timeout(
                    std::time::Duration::from_secs(60),
                    tokio::task::spawn_blocking(move || {
                        crate::media::video::first_frame_builtin(&p)
                    }),
                )
                .await
                .ok()
                .and_then(|r| r.ok())
                .flatten();
                if let Some(ref frame) = frame {
                    let poster = crate::media::video::poster_path_for(&path);
                    if !crate::media::video::poster_fresh(&path, &poster) {
                        crate::media::video::save_poster_jpeg(frame, &poster);
                    }
                    Some(crate::ai::phash::phash(frame))
                } else {
                    None
                }
            }
            None => None,
        };
        let mut near_duplicate_of = String::new();
        if let Some(ph) = phash {
            if let Some((other, dist)) = self
                .db
                .find_near_phash(ph, crate::ai::phash::MAX_HAMMING)
                .into_iter()
                .find(|(p, _)| p != &path.to_string_lossy().to_string())
            {
                near_duplicate_of = other.clone();
                tracing::info!(
                    "похожее изображение: {} ≈ {other} (расстояние {dist})",
                    path.display()
                );
            }
        }
        self.db.record_download(
            chat_label,
            message_id,
            &path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            size as i64,
            &sha,
            &path.to_string_lossy(),
            phash.map(|h| h as i64),
            &near_duplicate_of,
        );

        let mut yandex_path = String::new();
        if self.yandex_auto && !self.yandex_token.is_empty() && !duplicate {
            let remote = format!(
                "{}/{}",
                self.yandex_folder.trim_end_matches('/'),
                path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            );
            // Семафор: не больше 4 параллельных заливок.
            let _permit = upload_semaphore().acquire().await;
            let token = self.yandex_token.clone();
            let local = path.clone();
            let remote_c = remote.clone();
            let progress_tx = self.progress_tx.clone();
            let local_key = path.to_string_lossy().to_string();
            let progress_cb = move |done: u64, total: u64| {
                if let Some(tx) = &progress_tx {
                    let _ = tx.send(BgEvent::UploadProgress {
                        local_path: local_key.clone(),
                        done,
                        total,
                    });
                }
            };
            match yandex::upload_file_with_progress(&token, &local, &remote_c, progress_cb).await {
                Ok(_) => {
                    yandex_path = remote.clone();
                    self.db.mark_yandex(&path.to_string_lossy(), &remote);
                }
                Err(e) => tracing::warn!("Yandex upload failed: {e:#}"),
            }
        }

        let mut calibre_imported = false;
        if self.calibre_auto && utils::is_ebook(&path) {
            match calibre::import_book(&self.calibre_lib, &path, self.calibre_delete).await {
                Ok(calibre::ImportOutcome::Imported { .. }) => {
                    calibre_imported = true;
                    self.db.mark_calibre(&path.to_string_lossy());
                }
                Ok(calibre::ImportOutcome::Duplicate { .. }) => {
                    calibre_imported = false;
                    tracing::info!("calibre: дубль, пропуск");
                }
                Ok(calibre::ImportOutcome::Skipped(reason)) => {
                    tracing::info!("calibre skipped: {reason}");
                }
                Err(e) => tracing::warn!("calibre import failed: {e:#}"),
            }
        }

        Ok(DownloadReport {
            local_path: path,
            sha256: sha,
            duplicate,
            yandex_path,
            calibre_imported,
            phash,
            near_duplicate_of,
        })
    }
}

#[derive(Debug)]
pub struct DownloadReport {
    pub local_path: PathBuf,
    pub sha256: String,
    pub duplicate: bool,
    pub yandex_path: String,
    pub calibre_imported: bool,
    pub phash: Option<u64>,
    pub near_duplicate_of: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AutodownloadRule, DownloaderSettings, MediaKind};

    fn test_downloader(dir: &std::path::Path) -> (AutoDownloader, Arc<Db>) {
        let db = Arc::new(Db::open_at(dir.join("state.db")).unwrap());
        let settings = DownloaderSettings {
            download_dir: dir.join("dl"),
            auto_download_protected: true,
            rules: vec![AutodownloadRule {
                chat: "*".into(),
                kinds: vec![MediaKind::Any],
                max_size_mb: 1500,
                enabled: true,
                bypass_protected: true,
            }],
            parallel_chunks: 2,
            cluster_max_dist: 6,
        };
        let dl = AutoDownloader::new(
            settings,
            String::new(),
            "/Telecomfort".into(),
            false,
            String::new(),
            false,
            false,
            Arc::clone(&db),
        );
        (dl, db)
    }

    #[tokio::test]
    async fn download_many_parallel_mock() {
        let dir = tempfile::tempdir().unwrap();
        let (dl, db) = test_downloader(dir.path());
        let tg = Arc::new(crate::telegram::MockTelegram);
        let jobs: Vec<DownloadJob> = [101, 102, 103]
            .into_iter()
            .map(|id| DownloadJob {
                chat_id: 1,
                chat_label: "Test".into(),
                message_id: id,
                file_name: None,
                media_kind: None,
                file_size: None,
            })
            .collect();
        // file_name/media_kind неизвестны → classify даст Document, правило Any пропустит.
        let mut results = download_many(&dl, tg, jobs, 2).await;
        results.sort_by_key(|(j, _)| j.message_id);
        assert_eq!(results.len(), 3);
        for (job, res) in &results {
            let rep = res
                .as_ref()
                .unwrap_or_else(|e| panic!("job {}: {e:#}", job.message_id));
            assert!(rep.local_path.exists(), "{}", rep.local_path.display());
            assert!(!rep.sha256.is_empty());
        }
        // Все три записаны в SQLite.
        assert_eq!(db.recent_downloads(10).len(), 3);
    }

    #[tokio::test]
    async fn download_many_empty_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let (dl, _db) = test_downloader(dir.path());
        let tg = Arc::new(crate::telegram::MockTelegram);
        let out = download_many(&dl, tg, vec![], 3).await;
        assert!(out.is_empty());
    }
}

/// Одно задание на скачивание для `download_many`.
#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub chat_id: i64,
    pub chat_label: String,
    pub message_id: i32,
    pub file_name: Option<String>,
    pub media_kind: Option<String>,
    pub file_size: Option<i64>,
}

/// Скачать несколько файлов параллельно (до `max_concurrent_files`
/// одновременно). Внутри каждый файл уже качается в N MTProto-чанков,
/// так что нагрузка распределяется по соединениям пула к разным DC —
/// это и есть multi-DC: файлы физически лежат на разных дата-центрах
/// Telegram, `DcAffinity` + auth-перенос разруливают migrate/auth
/// на каждый из них. Возвращает по результату на каждое задание.
pub async fn download_many(
    downloader: &AutoDownloader,
    tg: std::sync::Arc<dyn TelegramService>,
    jobs: Vec<DownloadJob>,
    max_concurrent_files: usize,
) -> Vec<(DownloadJob, anyhow::Result<DownloadReport>)> {
    use std::sync::Arc;
    let sem = Arc::new(tokio::sync::Semaphore::new(
        max_concurrent_files.clamp(1, 16),
    ));
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        let dl = downloader.clone();
        let tg = Arc::clone(&tg);
        let sem = Arc::clone(&sem);
        set.spawn(async move {
            // Семафор ограничивает число одновременных файлов.
            let _permit = sem.acquire_owned().await.ok();
            let res = dl
                .download_one(
                    tg.as_ref(),
                    job.chat_id,
                    &job.chat_label,
                    job.message_id,
                    job.file_name.clone(),
                    job.media_kind.clone(),
                    job.file_size,
                )
                .await;
            (job, res)
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok(pair) => out.push(pair),
            Err(e) => tracing::error!("download_many: задача паниковала: {e}"),
        }
    }
    out
}
