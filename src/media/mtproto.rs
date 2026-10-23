//! Параллельная докачка файлов из MTProto чанками с resume и прогрессом.
//!
//! Штатный `Client::download_media` уже качает большие файлы в 4 потока,
//! но всегда с нуля и без прогресса. Здесь:
//! - `.part`-файл рядом с назначением (докачка после обрыва),
//! - N воркеров через `upload.getFile` с `precise: true` (как внутри grammers),
//! - shared `AtomicI64` на раздачу чанков + writer с `seek` (чанки приходят не по порядку),
//! - прогресс `done/total` после каждого чанка,
//! - малые файлы и неизвестный размер — fallback на `download_media`.

use grammers_client::media::{Downloadable, Media};
use grammers_client::{Client, InvocationError};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc::unbounded_channel;

/// Размер одного запроса к upload.getFile (как MAX_CHUNK_SIZE в grammers).
pub const CHUNK: u64 = 512 * 1024;
/// Файлы меньше порога качаем штатно (оверкилл параллелить).
pub const PARALLEL_THRESHOLD: u64 = 4 * 1024 * 1024;
/// Воркеров по умолчанию.
pub const DEFAULT_WORKERS: usize = 4;
/// FILE_MIGRATE_* — смена DC внутри воркера (как в grammers/files.rs).
const FILE_MIGRATE_ERROR: i32 = 303;

/// Разбить `[0, total)` на `workers` диапазонов (покрыто тестами).
pub fn chunk_ranges(total: u64, workers: usize) -> Vec<(u64, u64)> {
    let workers = workers.max(1) as u64;
    if total == 0 {
        return vec![];
    }
    let base = total / workers;
    let rem = total % workers;
    let mut out = Vec::new();
    let mut off = 0u64;
    for i in 0..workers {
        let len = base + if i < rem { 1 } else { 0 };
        if len == 0 {
            break;
        }
        out.push((off, off + len));
        off += len;
    }
    out
}

/// Стартовая позиция resume: усечение хвоста `.part` до границы чанка
/// (хвост мог быть записан последовательным фолбэком некратно чанку).
pub fn resume_start(existing: u64) -> u64 {
    existing / CHUNK * CHUNK
}

/// Путь `.part`-файла для докачки.
pub fn part_path_for(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Общая DC-привязка одного файла для multi-DC докачки.
///
/// Файл в Telegram физически лежит на одном DC, но заранее неизвестно на
/// каком: сервер сообщает это через `FILE_MIGRATE_*`. Первый воркер, поймавший
/// migrate, публикует правильный DC — остальные чанки сразу идут туда без
/// лишних roundtrip'ов. Разные файлы при этом естественно висят на разных
/// DC, и пул соединений grammers качает их одновременно (см.
/// `AutoDownloader::download_many`).
#[derive(Debug, Clone)]
pub struct DcAffinity {
    current: Arc<AtomicI32>,
}

impl DcAffinity {
    pub fn new(home_dc: i32) -> Self {
        Self { current: Arc::new(AtomicI32::new(home_dc)) }
    }

    pub fn get(&self) -> i32 {
        self.current.load(Ordering::Relaxed)
    }

    pub fn publish(&self, dc: i32) {
        self.current.store(dc, Ordering::Relaxed);
    }
}

/// Перенести авторизацию на другой DC (аналог приватного
/// `Client::copy_auth_to_dc` в grammers): `auth.exportAuthorization`
/// на домашнем DC + `auth.importAuthorization` на целевом.
/// Идемпотентно в рамках одного файла (кэш `done`).
async fn ensure_auth_in_dc(
    client: &Client,
    done: &Arc<tokio::sync::Mutex<HashSet<i32>>>,
    dc: i32,
) -> Result<(), InvocationError> {
    {
        let g = done.lock().await;
        if g.contains(&dc) {
            return Ok(());
        }
    }
    let exported = client
        .invoke(&grammers_client::tl::functions::auth::ExportAuthorization { dc_id: dc })
        .await?;
    let grammers_client::tl::enums::auth::ExportedAuthorization::Authorization(auth) =
        exported;
    client
        .invoke_in_dc(
            dc,
            &grammers_client::tl::functions::auth::ImportAuthorization {
                id: auth.id,
                bytes: auth.bytes,
            },
        )
        .await?;
    done.lock().await.insert(dc);
    Ok(())
}

/// Скачать `media` в `dest`: параллельные чанки + resume + прогресс.
/// Возвращает финальный путь `dest`.
pub async fn download_parallel(
    client: &Client,
    media: &Media,
    dest: &Path,
    workers: usize,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<PathBuf, anyhow::Error> {
    let total = match media.size() {
        Some(t) if t > 0 => t as u64,
        _ => {
            // Размер неизвестен — штатный путь.
            client
                .download_media(media, dest)
                .await
                .map_err(|e| anyhow::anyhow!("download_media: {e}"))?;
            on_progress(1, 1);
            return Ok(dest.to_path_buf());
        }
    };
    // Малый файл без недокачанного хвоста — штатно, быстро.
    let part = part_path_for(dest);
    let existing_raw = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if total < PARALLEL_THRESHOLD && existing_raw == 0 && !dest.exists() {
        client
            .download_media(media, dest)
            .await
            .map_err(|e| anyhow::anyhow!("download_media: {e}"))?;
        on_progress(total, total);
        return Ok(dest.to_path_buf());
    }
    if dest.exists() {
        if let Ok(m) = tokio::fs::metadata(dest).await {
            if m.len() == total {
                on_progress(total, total);
                return Ok(dest.to_path_buf());
            }
        }
        let _ = tokio::fs::remove_file(dest).await;
    }

    let location = match media.to_raw_input_location() {
        Some(l) => l,
        None => {
            client
                .download_media(media, dest)
                .await
                .map_err(|e| anyhow::anyhow!("download_media: {e}"))?;
            on_progress(total, total);
            return Ok(dest.to_path_buf());
        }
    };
    let home_dc = match media.dc_id() {
        Some(dc) => dc,
        None => {
            // Без DC не знаем, куда слать getFile — штатный путь сам разберётся.
            client
                .download_media(media, dest)
                .await
                .map_err(|e| anyhow::anyhow!("download_media: {e}"))?;
            on_progress(total, total);
            return Ok(dest.to_path_buf());
        }
    };

    // Resume: усекаем хвост до границы чанка.
    let mut start = resume_start(existing_raw.min(total));
    if existing_raw > total {
        let _ = tokio::fs::remove_file(&part).await;
        start = 0;
    }
    if start == total && total > 0 {
        tokio::fs::rename(&part, dest).await?;
        on_progress(total, total);
        return Ok(dest.to_path_buf());
    }

    // Предвыделяем место.
    {
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&part)
            .await?;
        f.set_len(total).await?;
        // Усечение хвоста, если part длиннее start (был обрезан выше логически).
        let _ = f;
        if existing_raw != start {
            let f2 = std::fs::OpenOptions::new().write(true).open(&part)?;
            f2.set_len(start)?;
        }
    }

    on_progress(start, total);
    let next_part = Arc::new(AtomicI64::new((start / CHUNK) as i64));
    let (tx, mut rx) = unbounded_channel::<(u64, Vec<u8>)>();
    let workers = workers.clamp(1, 8);
    // Multi-DC: общий текущий DC + кэш «авторизация уже перенесена».
    let affinity = DcAffinity::new(home_dc);
    let auth_done = Arc::new(tokio::sync::Mutex::new(HashSet::new()));

    let mut tasks = Vec::new();
    for _ in 0..workers {
        let location = location.clone();
        let tx = tx.clone();
        let next_part = Arc::clone(&next_part);
        let affinity = affinity.clone();
        let auth_done = Arc::clone(&auth_done);
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            let mut migrates = 0u8;
            loop {
                let i = next_part.fetch_add(1, Ordering::Relaxed);
                let offset = i as u64 * CHUNK;
                if offset >= total {
                    break;
                }
                let mut got = 0u64;
                let want = (total - offset).min(CHUNK);
                let mut retries = 0u8;
                while got < want {
                    // Каждый чанк стартует с актуального DC: если соседний
                    // воркер уже словил FILE_MIGRATE, идём сразу туда.
                    let task_dc = affinity.get();
                    let req = grammers_client::tl::functions::upload::GetFile {
                        precise: true,
                        cdn_supported: false,
                        location: location.clone(),
                        offset: (offset + got) as i64,
                        limit: (want - got).min(CHUNK) as i32,
                    };
                    match client.invoke_in_dc(task_dc, &req).await {
                        Ok(grammers_client::tl::enums::upload::File::File(f)) => {
                            if f.bytes.is_empty() {
                                break; // EOF
                            }
                            got += f.bytes.len() as u64;
                            if tx.send((offset + got - f.bytes.len() as u64, f.bytes)).is_err() {
                                return Err(InvocationError::Io(std::io::Error::other(
                                    "writer gone",
                                )));
                            }
                        }
                        Ok(grammers_client::tl::enums::upload::File::CdnRedirect(_)) => {
                            return Err(InvocationError::Io(std::io::Error::other(
                                "cdn redirect: запрошен cdn_supported=false, сервер всё равно редиректит",
                            )));
                        }
                        Err(InvocationError::Rpc(err)) if err.code == FILE_MIGRATE_ERROR => {
                            // Сервер сказал правильный DC: публикуем для всех
                            // воркеров и переспрашиваем тот же чанк (ретраи не тратим).
                            if let Some(dc) = err.value {
                                affinity.publish(dc as i32);
                            }
                            migrates += 1;
                            if migrates > 20 {
                                return Err(InvocationError::Io(std::io::Error::other(
                                    "FILE_MIGRATE зациклился",
                                )));
                            }
                            continue;
                        }
                        Err(InvocationError::Rpc(ref err)) if err.name == "AUTH_KEY_UNREGISTERED" => {
                            // Файл на DC без нашей авторизации: переносим
                            // export/import и повторяем чанк (как в grammers).
                            if let Err(e) = ensure_auth_in_dc(&client, &auth_done, task_dc).await {
                                return Err(e);
                            }
                            continue;
                        }
                        Err(e) if retries < 3 => {
                            retries += 1;
                            tracing::debug!("mtproto chunk retry {retries}/3 at {offset}: {e}");
                            tokio::time::sleep(std::time::Duration::from_millis(300 * retries as u64)).await;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            Ok::<(), InvocationError>(())
        }));
    }
    drop(tx);

    // Writer: чанки могут приходить не по порядку — seek по offset.
    let mut file = tokio::fs::OpenOptions::new().write(true).open(&part).await?;
    let mut writer_done: u64 = start;
    let mut task_error: Option<InvocationError> = None;
    // Позиция курсора для оптимизации последовательной записи.
    let mut pos = start;
    while let Some((offset, data)) = rx.recv().await {
        if offset != pos {
            file.seek(std::io::SeekFrom::Start(offset)).await.map_err(InvocationError::Io)?;
        }
        file.write_all(&data).await.map_err(InvocationError::Io)?;
        pos = offset + data.len() as u64;
        writer_done += data.len() as u64;
        on_progress(writer_done.min(total), total);
    }
    drop(file);

    for t in tasks {
        match t.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                task_error = Some(e);
                break;
            }
            Err(e) => {
                task_error = Some(InvocationError::Io(std::io::Error::other(format!(
                    "worker panicked: {e}"
                ))));
                break;
            }
        }
    }
    if let Some(e) = task_error {
        anyhow::bail!("параллельная докачка не удалась: {e}");
    }

    let got = tokio::fs::metadata(&part).await?.len();
    if got != total {
        anyhow::bail!("неполный файл: {got}/{total} байт (part сохранён, докачайте повторным запуском)");
    }
    tokio::fs::rename(&part, dest).await?;
    on_progress(total, total);
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_cover_exactly() {
        let r = chunk_ranges(100, 4);
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].0, 0);
        assert_eq!(r.last().unwrap().1, 100);
        // Непрерывность и сумма
        let mut sum = 0u64;
        let mut prev_end = 0u64;
        for (s, e) in r {
            assert_eq!(s, prev_end);
            sum += e - s;
            prev_end = e;
        }
        assert_eq!(sum, 100);
    }

    #[test]
    fn ranges_edge_cases() {
        assert!(chunk_ranges(0, 4).is_empty());
        // Меньше воркеров, чем чанков
        let r = chunk_ranges(3, 8);
        assert_eq!(r.len(), 3);
        assert_eq!(r.last().unwrap().1, 3);
        let r = chunk_ranges(10 * 1024 * 1024, 4);
        assert_eq!(r.len(), 4);
        assert_eq!(r.last().unwrap().1, 10 * 1024 * 1024);
    }

    #[test]
    fn resume_truncates_to_chunk() {
        assert_eq!(resume_start(0), 0);
        assert_eq!(resume_start(CHUNK), CHUNK);
        assert_eq!(resume_start(CHUNK + 123), CHUNK);
        assert_eq!(resume_start(3 * CHUNK + CHUNK - 1), 3 * CHUNK);
    }

    #[test]
    fn part_path_appends_suffix() {
        let d = Path::new("/tmp/lecture.mp4");
        assert_eq!(part_path_for(d), PathBuf::from("/tmp/lecture.mp4.part"));
    }

    #[test]
    fn dc_affinity_publish_follow() {
        let a = DcAffinity::new(2);
        assert_eq!(a.get(), 2);
        let b = a.clone();
        // Воркер публикует migrate — второй воркер видит сразу.
        a.publish(4);
        assert_eq!(b.get(), 4);
        a.publish(5);
        assert_eq!(a.get(), 5);
    }
}
