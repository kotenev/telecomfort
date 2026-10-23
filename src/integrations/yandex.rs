//! Яндекс Диск через WebDAV: https://webdav.yandex.ru
//!
//! Нужен OAuth-токен (https://yandex.ru/dev/disk/poligon/).
//! Загрузка: PUT /<remote_path>, создание папок: MKCOL.
//!
//! Большие файлы (видео из закрытых групп, 1ГБ+) заливаются
//! **стримингом** — без чтения всего файла в RAM, с прогрессом,
//! ретраями и лимитом параллельных заливок (см. `AutoDownloader`).

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("telecomfort/0.2")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .expect("reqwest client")
}

/// Порог стриминга: файлы больше — не читаем целиком, а стримим чанками.
pub const STREAM_THRESHOLD: u64 = 8 * 1024 * 1024;
/// Размер чанка стриминга.
pub const CHUNK_SIZE: usize = 1024 * 1024;

/// План чанков для файла: (offset, len) — чистая функция, покрыта тестами.
pub fn chunk_plan(file_size: u64, chunk_size: u64) -> Vec<(u64, u64)> {
    if file_size == 0 || chunk_size == 0 {
        return vec![];
    }
    let mut out = Vec::new();
    let mut off = 0u64;
    while off < file_size {
        let len = (file_size - off).min(chunk_size);
        out.push((off, len));
        off += len;
    }
    out
}

pub async fn ensure_folder(token: &str, remote_folder: &str) -> Result<()> {
    if token.is_empty() {
        anyhow::bail!("пустой OAuth-токен Яндекс Диска");
    }
    let url = format!(
        "https://webdav.yandex.ru{}",
        ensure_leading_slash(remote_folder)
    );
    let resp = client()
        .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url)
        .bearer_auth(token)
        .send()
        .await
        .context("MKCOL yandex")?;
    // 201 создан, 405 уже существует — оба ок
    if resp.status().as_u16() != 201 && resp.status().as_u16() != 405 {
        let code = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("MKCOL {url}: {code} {text}");
    }
    Ok(())
}

/// Простая загрузка без прогресса (малые файлы — одним PUT в память).
pub async fn upload_file(token: &str, local: &Path, remote_path: &str) -> Result<()> {
    upload_file_with_progress(token, local, remote_path, |_, _| {}).await
}

/// Загрузка с прогрессом `on_progress(done, total)`, ретраями и стримингом.
/// Параллельность ограничивается вызывающим через семафор (4 по умолчанию).
pub async fn upload_file_with_progress(
    token: &str,
    local: &Path,
    remote_path: &str,
    on_progress: impl Fn(u64, u64) + Send + Sync + 'static,
) -> Result<()> {
    if token.is_empty() {
        anyhow::bail!("пустой OAuth-токен Яндекс Диска");
    }
    if let Some(parent) = Path::new(remote_path).parent().and_then(|p| p.to_str()) {
        if !parent.is_empty() && parent != "/" {
            let _ = ensure_folder(token, parent).await;
        }
    }
    let meta = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("stat {}", local.display()))?;
    let total = meta.len();
    if total == 0 {
        anyhow::bail!("пустой файл: {}", local.display());
    }

    let on_progress = Arc::new(on_progress);
    let mut last_err = anyhow::anyhow!("not attempted");
    for attempt in 1..=3 {
        let res = if total <= STREAM_THRESHOLD {
            let cb = Arc::clone(&on_progress);
            put_whole(token, local, remote_path, total, move |d, t| cb(d, t)).await
        } else {
            let cb = Arc::clone(&on_progress);
            put_streaming(token, local, remote_path, total, move |d, t| cb(d, t)).await
        };
        match res {
            Ok(_) => {
                on_progress(total, total);
                tracing::info!("загружено на Яндекс Диск: {remote_path} ({} байт)", total);
                return Ok(());
            }
            Err(e) => {
                // 4xx (кроме 429/408) не ретраим — это не сеть, а запрос.
                let s = format!("{e:#}");
                let non_retryable = (s.contains(" 400") || s.contains(" 401")
                    || s.contains(" 403")
                    || s.contains(" 404"))
                    && !s.contains("429")
                    && !s.contains("408");
                last_err = e;
                if non_retryable {
                    break;
                }
                tracing::warn!("Яндекс PUT попытка {attempt}/3: {last_err:#}");
                if attempt < 3 {
                    tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
                }
            }
        }
    }
    Err(last_err.context(format!("PUT {remote_path} не удался за 3 попытки")))
}

async fn put_whole(
    token: &str,
    local: &Path,
    remote_path: &str,
    total: u64,
    on_progress: impl Fn(u64, u64) + Send + Sync + 'static,
) -> Result<()> {
    let url = format!("https://webdav.yandex.ru{}", ensure_leading_slash(remote_path));
    let bytes = tokio::fs::read(local).await.with_context(|| format!("read {}", local.display()))?;
    on_progress(bytes.len() as u64, total);
    let resp = client()
        .put(&url)
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .context("PUT yandex")?;
    check_put(&url, resp).await
}

async fn put_streaming(
    token: &str,
    local: &Path,
    remote_path: &str,
    total: u64,
    on_progress: impl Fn(u64, u64) + Send + Sync + 'static,
) -> Result<()> {
    use tokio_util::io::ReaderStream;
    let url = format!("https://webdav.yandex.ru{}", ensure_leading_slash(remote_path));
    let done = Arc::new(AtomicU64::new(0));
    let file = tokio::fs::File::open(local)
        .await
        .with_context(|| format!("open {}", local.display()))?;
    let stream = ReaderStream::with_capacity(file, CHUNK_SIZE);
    let cb: Arc<dyn Fn(u64, u64) + Send + Sync> = Arc::new(on_progress);
    let progress_stream = progress_wrap(stream, Arc::clone(&done), total, cb);
    let body = reqwest::Body::wrap_stream(progress_stream);
    let resp = client()
        .put(&url)
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", total.to_string())
        .body(body)
        .send()
        .await
        .context("PUT yandex (stream)")?;
    check_put(&url, resp).await
}

/// Оборачивает byte-stream счётчиком: каждый чанк двигает прогресс.
fn progress_wrap<S>(
    inner: S,
    done: Arc<AtomicU64>,
    total: u64,
    cb: Arc<dyn Fn(u64, u64) + Send + Sync>,
) -> impl futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send
where
    S: futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin + 'static,
{
    async_stream::stream! {
        use futures::StreamExt;
        let mut inner = inner;
        while let Some(chunk) = inner.next().await {
            match chunk {
                Ok(b) => {
                    let d = done.fetch_add(b.len() as u64, Ordering::Relaxed) + b.len() as u64;
                    cb(d.min(total), total);
                    yield Ok(b);
                }
                Err(e) => {
                    yield Err(e);
                    break;
                }
            }
        }
    }
}

async fn check_put(url: &str, resp: reqwest::Response) -> Result<()> {
    if !resp.status().is_success() {
        let code = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("PUT {url}: {code} {text}");
    }
    Ok(())
}

fn ensure_leading_slash(s: &str) -> String {
    if s.starts_with('/') {
        encode_path(s)
    } else {
        encode_path(&format!("/{s}"))
    }
}

fn encode_path(p: &str) -> String {
    p.split('/').map(urlencoding).collect::<Vec<_>>().join("/")
}

fn urlencoding(seg: &str) -> String {
    let mut out = String::new();
    for ch in seg.chars() {
        if ch.is_ascii_alphanumeric() || "-_.~".contains(ch) {
            out.push(ch);
        } else {
            for b in ch.to_string().as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_exactly() {
        assert_eq!(chunk_plan(0, 1024), vec![]);
        assert_eq!(chunk_plan(100, 0), vec![]);
        assert_eq!(chunk_plan(10, 4), vec![(0, 4), (4, 4), (8, 2)]);
        assert_eq!(chunk_plan(8, 8), vec![(0, 8)]);
        let plan = chunk_plan(10 * 1024 * 1024 + 5, 1024 * 1024);
        assert_eq!(plan.len(), 11);
        let sum: u64 = plan.iter().map(|(_, l)| l).sum();
        assert_eq!(sum, 10 * 1024 * 1024 + 5);
    }

    #[test]
    fn path_encoding_keeps_slashes() {
        assert_eq!(ensure_leading_slash("/Telecomfort/книга 1.epub"), "/Telecomfort/%D0%BA%D0%BD%D0%B8%D0%B3%D0%B0%201.epub");
    }
}
