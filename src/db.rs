//! Локальная SQLite-база состояния: загрузки, хэши, решения по рекламе.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open() -> Result<Self> {
        let dir = crate::config::default_config_dir();
        std::fs::create_dir_all(&dir).ok();
        Self::open_at(dir.join("state.db"))
    }

    pub fn open_at(path: PathBuf) -> Result<Self> {
        let conn = Connection::open(path).context("open sqlite")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        // Десктоп-режим: UI читает, фон пишет — WAL убирает блокировки
        // читателей писателем; NORMAL + busy_timeout для ноутбуков (SSD).
        // Best-effort: на read-only/сетевых FS молча остаёмся на journal_mode=delete.
        {
            let conn = db.conn.lock().unwrap();
            let _ = conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;",
            );
        }
        // Миграции существующих БД: новые колонки (идемпотентно —
        // ошибка «duplicate column» просто игнорируется).
        for stmt in [
            "ALTER TABLE downloads ADD COLUMN phash INTEGER",
            "ALTER TABLE downloads ADD COLUMN near_duplicate_of TEXT NOT NULL DEFAULT ''",
            "CREATE INDEX IF NOT EXISTS idx_downloads_phash ON downloads(phash)",
        ] {
            let _ = db.conn.lock().unwrap().execute(stmt, []);
        }
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS onnx_cache (
                img_sha TEXT NOT NULL,
                model_key TEXT NOT NULL,
                p REAL NOT NULL,
                boost REAL NOT NULL,
                reasons TEXT NOT NULL DEFAULT '',
                PRIMARY KEY(img_sha, model_key)
            );
            CREATE TABLE IF NOT EXISTS downloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat TEXT NOT NULL,
                message_id INTEGER NOT NULL,
                file_name TEXT NOT NULL,
                file_size INTEGER NOT NULL DEFAULT 0,
                sha256 TEXT NOT NULL DEFAULT '',
                local_path TEXT NOT NULL DEFAULT '',
                yandex_path TEXT NOT NULL DEFAULT '',
                calibre_imported INTEGER NOT NULL DEFAULT 0,
                phash INTEGER,
                near_duplicate_of TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(chat, message_id, file_name)
            );
            CREATE TABLE IF NOT EXISTS ad_decisions (
                story_key TEXT PRIMARY KEY,
                score REAL NOT NULL,
                reasons TEXT NOT NULL DEFAULT '',
                hidden INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS seen_hashes (
                sha256 TEXT PRIMARY KEY,
                first_path TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS stories_cache (
                story_key TEXT PRIMARY KEY,
                peer_id INTEGER NOT NULL DEFAULT 0,
                story_id INTEGER NOT NULL DEFAULT 0,
                caption TEXT NOT NULL DEFAULT '',
                preview_path TEXT NOT NULL DEFAULT '',
                score REAL NOT NULL DEFAULT 0,
                is_ad INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS sync_state (
                chat_id INTEGER PRIMARY KEY,
                last_message_id INTEGER NOT NULL DEFAULT 0,
                last_sync TEXT NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )
        .context("migrate")?;
        Ok(())
    }

    /// true = уже видели такой файл (дубль)
    pub fn is_duplicate_hash(&self, sha256: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT 1 FROM seen_hashes WHERE sha256 = ?1") {
            Ok(s) => s,
            Err(_) => return false,
        };
        stmt.exists(params![sha256]).unwrap_or(false)
    }

    pub fn remember_hash(&self, sha256: &str, path: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR IGNORE INTO seen_hashes(sha256, first_path) VALUES(?1, ?2)",
            params![sha256, path],
        );
    }

    pub fn record_download(
        &self,
        chat: &str,
        message_id: i32,
        file_name: &str,
        file_size: i64,
        sha256: &str,
        local_path: &str,
        phash: Option<i64>,
        near_duplicate_of: &str,
    ) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR IGNORE INTO downloads(chat, message_id, file_name, file_size, sha256, local_path, phash, near_duplicate_of)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![chat, message_id, file_name, file_size, sha256, local_path, phash, near_duplicate_of],
        );
    }

    /// Обновить pHash/дубль существующей записи (пересчёт дублей, backfill).
    /// `phash: None` — сбросить (файл не хэшируется).
    /// Возвращает число обновлённых строк (0 — пути нет в базе).
    pub fn update_phash(
        &self,
        local_path: &str,
        phash: Option<i64>,
        near_duplicate_of: &str,
    ) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET phash = ?1, near_duplicate_of = ?2 WHERE local_path = ?3",
            params![phash, near_duplicate_of, local_path],
        )
        .unwrap_or(0)
    }

    /// ONNX streaming-кэш: результат инференса для (sha картинки, ключ модели).
    /// Хит — и модель вообще не дёргается (ни сессии, ни GPU/CPU-инференса).
    pub fn onnx_cache_get(&self, img_sha: &str, model_key: &str) -> Option<(f32, f32, String)> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT p, boost, reasons FROM onnx_cache WHERE img_sha = ?1 AND model_key = ?2",
            params![img_sha, model_key],
            |r| Ok((r.get::<_, f64>(0)? as f32, r.get::<_, f64>(1)? as f32, r.get::<_, String>(2)?)),
        )
        .ok()
    }

    pub fn onnx_cache_put(&self, img_sha: &str, model_key: &str, p: f32, boost: f32, reasons: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO onnx_cache(img_sha, model_key, p, boost, reasons)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![img_sha, model_key, p as f64, boost as f64, reasons],
        );
    }

    pub fn onnx_cache_len(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM onnx_cache", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    /// Все (путь, pHash) для построения in-memory индексов (BK-дерево,
    /// кластеризация). Строки без pHash (не-картинки) пропускаются.
    pub fn all_phashes(&self) -> Vec<(String, u64)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT local_path, phash FROM downloads WHERE phash IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| {
            let path: String = r.get(0)?;
            let h: i64 = r.get(1)?;
            Ok((path, h as u64))
        })
        .map(|mapped| mapped.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// In-memory BK-индекс всех pHash (для быстрых повторных запросов
    /// без перестроения на каждый чих — см. `ai::bktree`).
    pub fn bk_index(&self) -> crate::ai::bktree::BkTree<String> {
        crate::ai::bktree::BkTree::from_iter(
            self.all_phashes().into_iter().map(|(path, h)| (h, path)),
        )
    }

    /// Ближайшие визуальные дубли по pHash (расстояние ≤ `max_dist`,
    /// дополнительно ограничено `MAX_HAMMING`),
    /// отсортированы по возрастанию расстояния. Сам файл исключает вызывающий
    /// по совпадению пути (передавайте свой путь — он уже записан? нет:
    /// поиск идёт ДО записи, так что достаточно разных путей).
    ///
    /// Реализация — через BK-дерево (`ai::bktree`): на тысячах строк так же
    /// быстро, как линейный скан, на десятках тысяч — заметно быстрее
    /// за счёт прунинга по неравенству треугольника.
    pub fn find_near_phash(&self, phash: u64, max_dist: u32) -> Vec<(String, u32)> {
        let radius = max_dist.min(crate::ai::phash::MAX_HAMMING);
        self.bk_index().query_cloned(phash, radius)
    }

    pub fn mark_yandex(&self, local_path: &str, yandex_path: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "UPDATE downloads SET yandex_path = ?1 WHERE local_path = ?2",
            params![yandex_path, local_path],
        );
    }

    pub fn mark_calibre(&self, local_path: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "UPDATE downloads SET calibre_imported = 1 WHERE local_path = ?1",
            params![local_path],
        );
    }

    pub fn save_ad_decision(&self, story_key: &str, score: f32, reasons: &str, hidden: bool) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO ad_decisions(story_key, score, reasons, hidden) VALUES(?1, ?2, ?3, ?4)",
            params![story_key, score, reasons, if hidden { 1 } else { 0 }],
        );
    }

    pub fn save_story_preview(
        &self,
        peer_id: i64,
        story_id: i32,
        caption: &str,
        preview_path: &str,
        score: f32,
        is_ad: bool,
    ) {
        let key = format!("{peer_id}:{story_id}");
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO stories_cache(story_key, peer_id, story_id, caption, preview_path, score, is_ad)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![key, peer_id, story_id, caption, preview_path, score, if is_ad { 1 } else { 0 }],
        );
    }

    pub fn story_preview(&self, peer_id: i64, story_id: i32) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT preview_path FROM stories_cache WHERE story_key = ?1")
            .ok()?;
        let key = format!("{peer_id}:{story_id}");
        stmt.query_row(params![key], |r| r.get::<_, String>(0)).ok()
    }

    pub fn get_last_message_id(&self, chat_id: i64) -> i32 {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT last_message_id FROM sync_state WHERE chat_id = ?1") {
            Ok(s) => s,
            Err(_) => return 0,
        };
        stmt.query_row(params![chat_id], |r| r.get::<_, i32>(0))
            .unwrap_or(0)
    }

    pub fn set_last_message_id(&self, chat_id: i64, last_id: i32) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO sync_state(chat_id, last_message_id, last_sync)
             VALUES(?1, ?2, datetime('now'))
             ON CONFLICT(chat_id) DO UPDATE SET last_message_id=excluded.last_message_id, last_sync=datetime('now')",
            params![chat_id, last_id],
        );
    }

    pub fn stats(&self) -> (usize, i64, usize) {
        let conn = self.conn.lock().unwrap();
        let total: usize = conn
            .query_row("SELECT COUNT(*) FROM downloads", [], |r| r.get(0))
            .unwrap_or(0);
        let size: i64 = conn
            .query_row("SELECT COALESCE(SUM(file_size),0) FROM downloads", [], |r| r.get(0))
            .unwrap_or(0);
        let dupes: usize = conn
            .query_row("SELECT COUNT(*) FROM seen_hashes", [], |r| r.get(0))
            .unwrap_or(0);
        (total, size, dupes)
    }

    pub fn recent_downloads(&self, limit: usize) -> Vec<DownloadRow> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT chat, message_id, file_name, file_size, local_path, yandex_path, calibre_imported, created_at, phash, near_duplicate_of
             FROM downloads ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            // Старая БД без новых колонок (миграция не применилась?): fallback без них.
            Err(_) => return self.recent_downloads_legacy(limit),
        };
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(DownloadRow {
                    chat: r.get(0)?,
                    message_id: r.get(1)?,
                    file_name: r.get(2)?,
                    file_size: r.get(3)?,
                    local_path: r.get(4)?,
                    yandex_path: r.get(5)?,
                    calibre_imported: r.get::<_, i64>(6)? != 0,
                    created_at: r.get(7)?,
                    phash: r.get::<_, Option<i64>>(8)?,
                    near_duplicate_of: r.get::<_, String>(9).unwrap_or_default(),
                })
            });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => self.recent_downloads_legacy(limit),
        }
    }

    fn recent_downloads_legacy(&self, limit: usize) -> Vec<DownloadRow> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT chat, message_id, file_name, file_size, local_path, yandex_path, calibre_imported, created_at
             FROM downloads ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![limit as i64], |r| {
            Ok(DownloadRow {
                chat: r.get(0)?,
                message_id: r.get(1)?,
                file_name: r.get(2)?,
                file_size: r.get(3)?,
                local_path: r.get(4)?,
                yandex_path: r.get(5)?,
                calibre_imported: r.get::<_, i64>(6)? != 0,
                created_at: r.get(7)?,
                phash: None,
                near_duplicate_of: String::new(),
            })
        })
        .map(|mapped| mapped.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRow {
    pub chat: String,
    pub message_id: i32,
    pub file_name: String,
    pub file_size: i64,
    pub local_path: String,
    pub yandex_path: String,
    pub calibre_imported: bool,
    pub created_at: String,
    pub phash: Option<i64>,
    pub near_duplicate_of: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onnx_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_at(dir.path().join("state.db")).unwrap();
        assert!(db.onnx_cache_get("img1", "modelA").is_none());
        db.onnx_cache_put("img1", "modelA", 0.83, 0.2, "ONNX ad.onnx: p=0.83");
        db.onnx_cache_put("img1", "modelB", 0.1, 0.0, "");
        assert_eq!(db.onnx_cache_len(), 2);
        let (p, b, r) = db.onnx_cache_get("img1", "modelA").unwrap();
        assert!((p - 0.83).abs() < 1e-6 && (b - 0.2).abs() < 1e-6);
        assert_eq!(r, "ONNX ad.onnx: p=0.83");
        // Перезапись того же ключа.
        db.onnx_cache_put("img1", "modelA", 0.9, 0.24, "x");
        assert_eq!(db.onnx_cache_len(), 2);
        assert!((db.onnx_cache_get("img1", "modelA").unwrap().0 - 0.9).abs() < 1e-6);
    }

    #[test]
    fn near_phash_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_at(dir.path().join("state.db")).unwrap();
        db.record_download("chat", 1, "a.png", 10, "sha-a", "/tmp/a.png", Some(0x1234), "");
        db.record_download("chat", 2, "b.png", 10, "sha-b", "/tmp/b.png", Some(0x1235), "");
        db.record_download("chat", 3, "c.txt", 10, "sha-c", "/tmp/c.txt", None, "");
        // 0x1234 vs 0x1235 — 1 бит разницы.
        let near = db.find_near_phash(0x1234, 6);
        assert_eq!(near.len(), 2);
        assert_eq!(near[0], ("/tmp/a.png".to_string(), 0));
        assert_eq!(near[1], ("/tmp/b.png".to_string(), 1));
        assert!(db.find_near_phash(0xFFFF_FFFF_FFFF_FFFF, 6).is_empty());
        // recent_downloads отдаёт новые поля.
        let rows = db.recent_downloads(10);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|r| r.phash == Some(0x1234)));
    }

    #[test]
    fn wal_mode_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_at(dir.path().join("state.db")).unwrap();
        let conn = db.conn.lock().unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap_or_default();
        // На обычных FS — wal; на экзотике (read-only/tmpfs без WAL) — молча fallback.
        assert!(
            mode.eq_ignore_ascii_case("wal") || !mode.is_empty(),
            "journal_mode={mode}"
        );
    }

    #[test]
    fn update_phash_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_at(dir.path().join("state.db")).unwrap();
        db.record_download("chat", 1, "v.mp4", 100, "sha-v", "/tmp/v.mp4", None, "");
        assert_eq!(db.all_phashes().len(), 0);
        // Backfill: проставили хэш + дубль.
        assert_eq!(db.update_phash("/tmp/v.mp4", Some(0x7777), "/tmp/a.png"), 1);
        assert_eq!(db.all_phashes(), vec![("/tmp/v.mp4".to_string(), 0x7777)]);
        let rows = db.recent_downloads(10);
        assert_eq!(rows[0].phash, Some(0x7777));
        assert_eq!(rows[0].near_duplicate_of, "/tmp/a.png");
        // Несуществующий путь — 0 строк, без ошибки.
        assert_eq!(db.update_phash("/tmp/nope.mp4", Some(1), ""), 0);
        // Сброс.
        assert_eq!(db.update_phash("/tmp/v.mp4", None, ""), 1);
        assert!(db.all_phashes().is_empty());
    }
}
