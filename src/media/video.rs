//! Видео-превью сторис: первый кадр mp4/mov/webm → JPG-постер.
//!
//! Цепочка (всё без тяжёлых ffmpeg-привязок в зависимостях):
//! 0. MTProto-тамб Telegram (см. backend_grammers: тамб сюжета качается как
//!    `poster.jpg` сразу вместе с видео — серверный кадр, без ffmpeg вообще).
//! 1. Постер `story_<peer>_<id>.poster.jpg` рядом с видео (кэш).
//! 2. Встроенный H.264-декодер (`media/h264.rs`, чистый Rust,
//!    `rusty_h264-decoder`): первый IDR-кадр из `moov/avcC + mdat`.
//!    Работает и для чужих mp4 без тамбов/обложек, без внешнего ffmpeg.
//! 3. Встроенный HEVC-декодер (`media/hevc.rs`, чистый Rust,
//!    `rusty_h265`): первый IRAP-кадр из `moov/hvcC + mdat` (hvc1/hev1).
//! 4. Встроенный AV1-декодер (`media/av1.rs`, чистый Rust,
//!    `rusty_av1d` без asm): первый показной кадр из `moov/av1C + mdat`
//!    (av01) или WebM/MKV (`CodecPrivate + SimpleBlock/Block`, свой
//!    EBML-парсер без зависимостей).
//! 5. Встроенная обложка `moov.udta.meta.ilst.covr.data` (чистый Rust,
//!    `extract_embedded_cover`, первые 16 МБ файла).
//! 5. Внешний `ffmpeg` (если установлен):
//!    `ffmpeg -y -ss 0.3 -i video -vframes 1 -q:v 4 -s 270x480 poster`.
//! 6. Fallback на градиент-плейсхолдер (см. ui::app).
//!
//! Так live-видеосторис показывают реальный кадр, а не «кинополосы»,
//! но сборка не требует libav/ffmpeg-sys.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn is_video_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_lowercase().as_str(), "mp4" | "mov" | "webm" | "mkv" | "avi"))
        .unwrap_or(false)
}

/// Путь к постеру для видео (тот же stem + `.poster.jpg`).
pub fn poster_path_for(video: &Path) -> PathBuf {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".into());
    video
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("{stem}.poster.jpg"))
}

/// Постер свежий, если существует, не пуст и не старше видео.
pub fn poster_fresh(video: &Path, poster: &Path) -> bool {
    let (Ok(vm), Ok(pm)) = (std::fs::metadata(video), std::fs::metadata(poster)) else {
        return false;
    };
    if pm.len() == 0 {
        return false;
    }
    let (Ok(vt), Ok(pt)) = (vm.modified(), pm.modified()) else {
        return pm.len() > 0;
    };
    pt >= vt
}

/// Встроенный поиск обложки в mp4/mov без внешних зависимостей:
/// `moov.udta.meta.ilst.covr.data` (iTunes-стиль; JPEG/PNG).
/// Сканирует первые 16 МБ (moov обычно в начале у faststart-файлов Telegram).
/// Возвращает декодированный кадр или `None`.
pub fn extract_embedded_cover(path: &Path) -> Option<image::DynamicImage> {
    const SCAN_LIMIT: u64 = 16 * 1024 * 1024;
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    let n = meta.len().min(SCAN_LIMIT) as usize;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; n];
    {
        use std::io::Read;
        let mut f = f;
        // Частичное чтение: достаточно, чтобы не тащить 1.5ГБ в RAM.
        let mut read = 0usize;
        while read < n {
            match f.read(&mut buf[read..]) {
                Ok(0) => break,
                Ok(k) => read += k,
                Err(_) => return None,
            }
        }
        buf.truncate(read);
    }
    find_covr_image(&buf).and_then(|bytes| image::load_from_memory(bytes).ok())
}

/// Поиск байтов картинки в боксах (чистая функция, покрыта тестами).
fn find_covr_image(buf: &[u8]) -> Option<&[u8]> {
    // Стек (end, in_covr): рекурсивный обход заменим явным стеком фреймов.
    parse_boxes(buf, false)
}

fn parse_boxes(full: &[u8], in_covr: bool) -> Option<&[u8]> {
    // Обход боксов верхнего уровня, в контейнеры заходим рекурсией.
    let mut off = 0usize;
    while off + 8 <= full.len() {
        let size32 = u32::from_be_bytes([full[off], full[off + 1], full[off + 2], full[off + 3]]) as u64;
        let tag = &full[off + 4..off + 8];
        let (hdr, payload_start) = if size32 == 1 {
            if off + 16 > full.len() {
                break;
            }
            let size64 = u64::from_be_bytes(full[off + 8..off + 16].try_into().unwrap());
            (size64, off + 16)
        } else if size32 == 0 {
            (full.len() as u64 - off as u64, off + 8)
        } else {
            (size32, off + 8)
        };
        if hdr < 8 || payload_start + (hdr as usize).saturating_sub(payload_start - off) > full.len() {
            break;
        }
        let end = off + hdr as usize;
        let mut payload = &full[payload_start..end];
        // `meta` — fullbox: первые 4 байта пропускаем.
        if tag == b"meta" {
            if payload.len() < 4 {
                break;
            }
            payload = &payload[4..];
        }
        if tag == b"data" && in_covr && payload.len() > 8 {
            let img = &payload[8..];
            if img.len() > 16
                && ((img[0] == 0xFF && img[1] == 0xD8)
                    || (img[0] == 0x89
                        && img[1] == b'P'
                        && img[2] == b'N'
                        && img[3] == b'G'))
            {
                return Some(img);
            }
        }
        let child_covr = in_covr || tag == b"covr";
        if matches!(tag, b"moov" | b"trak" | b"mdia" | b"minf" | b"udta" | b"meta" | b"ilst" | b"covr") {
            if let Some(found) = parse_boxes(payload, child_covr) {
                return Some(found);
            }
        }
        off = end;
    }
    None
}

/// Синхронная проверка наличия ffmpeg в PATH.
pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Извлечь первый кадр. Возвращает путь к постеру.
/// Порядок: встроенные H.264/HEVC/AV1 (чистый Rust) → ffmpeg (таймаут 20с).
/// Ошибка → вызывающий использует плейсхолдер.
pub async fn extract_poster(video: &Path, poster: &Path) -> anyhow::Result<PathBuf> {
    if poster_fresh(video, poster) {
        return Ok(poster.to_path_buf());
    }
    if let Some(parent) = poster.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    // Проверяем размер видео: пустой файл — сразу ошибка.
    let meta = tokio::fs::metadata(video).await?;
    if meta.len() == 0 {
        anyhow::bail!("пустое видео");
    }
    // Встроенные декодеры H.264/HEVC/AV1 (без ffmpeg/тамбов): CPU-bound, в blocking-пуле.
    {
        let v = video.to_path_buf();
        let p = poster.to_path_buf();
        let builtin_hit = tokio::task::spawn_blocking(move || {
            super::h264::poster_from_video(&v, &p)
                .or_else(|| super::hevc::poster_from_video(&v, &p))
                .or_else(|| super::av1::poster_from_video(&v, &p))
        })
        .await?;
        if let Some(done) = builtin_hit {
            tracing::debug!("постер из встроенного декодера: {}", done.display());
            return Ok(done);
        }
    }
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg("0.3")
        .arg("-i")
        .arg(video)
        .arg("-vframes")
        .arg("1")
        .arg("-q:v")
        .arg("4")
        .arg("-vf")
        .arg("scale=270:480:force_original_aspect_ratio=increase,crop=270:480")
        .arg(poster);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output()).await??;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!("ffmpeg: {}", if err.is_empty() { "non-zero exit".into() } else { err });
    }
    // Проверяем, что постер реально записался и это валидный JPEG.
    let data = tokio::fs::read(poster).await?;
    if data.len() < 1024 {
        anyhow::bail!("постер слишком маленький");
    }
    // Быстрая валидация через image (без паники).
    let poster_owned = poster.to_path_buf();
    let valid = tokio::task::spawn_blocking(move || image::open(&poster_owned).is_ok())
        .await
        .unwrap_or(false);
    if !valid {
        anyhow::bail!("постер не декодируется");
    }
    Ok(poster.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poster_path_naming() {
        let v = Path::new("/tmp/story_1_2.mp4");
        assert_eq!(poster_path_for(v), PathBuf::from("/tmp/story_1_2.poster.jpg"));
    }

    #[test]
    fn detects_video_ext() {
        assert!(is_video_path(Path::new("a.mp4")));
        assert!(is_video_path(Path::new("a.MOV")));
        assert!(!is_video_path(Path::new("a.jpg")));
        assert!(!is_video_path(Path::new("a.png")));
    }

    #[test]
    fn stale_poster_is_not_fresh() {
        // Несуществующие файлы — не fresh, без паники.
        assert!(!poster_fresh(Path::new("/nope/v.mp4"), Path::new("/nope/p.jpg")));
    }

    fn boxed(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        v.extend_from_slice(tag);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn covr_roundtrip() {
        // Собираем синтетический moov: udta.meta.ilst.covr.data(JPEG 2x2).
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([200, 30, 30]),
        ));
        // Кодируем JPEG через DynamicImage::write_to.
        let mut jpeg2 = Vec::new();
        {
            use std::io::Cursor;
            let mut cur = Cursor::new(&mut jpeg2);
            img.write_to(&mut cur, image::ImageFormat::Jpeg).unwrap();
        }
        assert!(jpeg2.len() > 16);
        let mut data_payload = vec![0u8; 8]; // type + locale
        data_payload.extend_from_slice(&jpeg2);
        let covr = boxed(b"covr", &boxed(b"data", &data_payload));
        let ilst = boxed(b"ilst", &covr);
        let mut meta_payload = vec![0u8; 4]; // fullbox header
        meta_payload.extend_from_slice(&ilst);
        let meta = boxed(b"meta", &meta_payload);
        let udta = boxed(b"udta", &meta);
        let moov = boxed(b"moov", &udta);
        let mut file = boxed(b"ftyp", b"isom");
        file.extend_from_slice(&moov);
        let found = find_covr_image(&file).expect("cover must be found");
        assert_eq!(found, jpeg2.as_slice());
        let decoded = image::load_from_memory(found).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
    }

    #[test]
    fn no_cover_returns_none() {
        let file = boxed(b"ftyp", b"isom");
        assert!(find_covr_image(&file).is_none());
        // data-бокс без covr-родителя игнорируется.
        let mut payload = vec![0u8; 8];
        payload.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let file2 = boxed(b"data", &payload);
        assert!(find_covr_image(&file2).is_none());
    }
}
