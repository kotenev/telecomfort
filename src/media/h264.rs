//! Встроенный H.264-декодер первого кадра (чистый Rust, без ffmpeg).
//!
//! Зачем: видеосторис из чужих mp4 часто приходят без серверного тамба
//! Telegram (`Document.thumbs` пуст) и без встроенной обложки `covr` —
//! тогда раньше оставался только внешний `ffmpeg` или плейсхолдер.
//! Теперь первый IDR-кадр декодируется прямо в приложении через
//! `rusty_h264-decoder` (pure Rust, без нативных библиотек):
//! `moov → avcC (SPS/PPS)` + `mdat (length-prefixed NAL)` → Annex B →
//! декодер → YUV420 → RGB → постер 270x480.
//!
//! Всё тяжёлое — за чистыми функциями с тестами; на битых файлах
//! возвращаем `None` без паники (декодер дополнительно обёрнут
//! в `catch_unwind`, т.к. вход — недоверенные байты из сети).

use std::path::{Path, PathBuf};

/// Сколько байт файла сканируем в поисках `moov`/`mdat` (moov у faststart-
/// файлов Telegram — в начале; лимит защищает от чтения 1.5ГБ в RAM).
pub const SCAN_LIMIT: u64 = 24 * 1024 * 1024;
/// Лимит собранного Annex B для декодера (первый IDR-кадр + SPS/PPS).
pub const ANNEXB_LIMIT: usize = 8 * 1024 * 1024;
/// Максимум NAL-юнитов в одном кадре-поcтере (защита от злонамеренных длин).
pub const MAX_NALS_PER_PICTURE: usize = 2000;

/// Параметры потока из бокса `avcC`.
#[derive(Debug, Clone)]
pub struct AvcConfig {
    /// Длина префикса NAL в `mdat` (обычно 4).
    pub length_size: usize,
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
}

/// Распарсить payload бокса `avcC` (без size+tag).
/// Формат ISO/IEC 14496-15 §5.2.4.1.2.
pub fn parse_avcc(data: &[u8]) -> Option<AvcConfig> {
    if data.len() < 7 || data[0] != 1 {
        return None;
    }
    let length_size = (data[4] & 0x03) as usize + 1;
    if !(1..=4).contains(&length_size) {
        return None;
    }
    let mut off = 5usize;
    let num_sps = (data[off] & 0x1F) as usize;
    off += 1;
    let mut sps = Vec::new();
    for _ in 0..num_sps {
        if off + 2 > data.len() {
            return None;
        }
        let len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
        off += 2;
        if off + len > data.len() || len < 4 {
            return None;
        }
        sps.push(data[off..off + len].to_vec());
        off += len;
    }
    if off >= data.len() || sps.is_empty() {
        return None;
    }
    let num_pps = data[off] as usize;
    off += 1;
    let mut pps = Vec::new();
    for _ in 0..num_pps {
        if off + 2 > data.len() {
            return None;
        }
        let len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
        off += 2;
        if off + len > data.len() || len == 0 {
            return None;
        }
        pps.push(data[off..off + len].to_vec());
        off += len;
    }
    if pps.is_empty() {
        return None;
    }
    Some(AvcConfig { length_size, sps, pps })
}

/// Тип NAL-юнита (младшие 5 бит заголовка).
pub fn nal_type(byte0: u8) -> u8 {
    byte0 & 0x1F
}

/// Итерация боксов верхнего уровня/контейнера: (tag, payload).
/// `header_skip` — пропуск fullbox-заголовка (stsd: 8, meta: 4).
pub fn boxes_in(data: &[u8], header_skip: usize) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    if header_skip >= data.len() {
        return out;
    }
    let mut off = header_skip;
    while off + 8 <= data.len() {
        let size32 = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        let tag: [u8; 4] = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        let (hdr, start) = if size32 == 1 {
            if off + 16 > data.len() {
                break;
            }
            let size64 = u64::from_be_bytes(data[off + 8..off + 16].try_into().unwrap()) as usize;
            (size64, off + 16)
        } else if size32 == 0 {
            (data.len() - off, off + 8)
        } else {
            (size32, off + 8)
        };
        if hdr < 8 || start + hdr.saturating_sub(start - off) > data.len() {
            break;
        }
        let end = off + hdr;
        out.push((tag, &data[start..end]));
        if end <= off {
            break;
        }
        off = end;
    }
    out
}

fn tag_eq(t: &[u8; 4], s: &[u8; 4]) -> bool {
    t == s
}

/// Найти `avcC` по пути moov→trak→mdia→minf→stbl→stsd→avc1/avc3→avcC.
pub fn find_avcc(data: &[u8]) -> Option<AvcConfig> {
    for (t, p) in boxes_in(data, 0) {
        if tag_eq(&t, b"moov") {
            if let Some(c) = find_avcc_in_moov(p) {
                return Some(c);
            }
        }
    }
    None
}

fn find_avcc_in_moov(moov: &[u8]) -> Option<AvcConfig> {
    for (t, p) in boxes_in(moov, 0) {
        if !tag_eq(&t, b"trak") {
            continue;
        }
        for (t, p) in boxes_in(p, 0) {
            if !tag_eq(&t, b"mdia") {
                continue;
            }
            for (t, p) in boxes_in(p, 0) {
                if !tag_eq(&t, b"minf") {
                    continue;
                }
                for (t, p) in boxes_in(p, 0) {
                    if !tag_eq(&t, b"stbl") {
                        continue;
                    }
                    for (t, p) in boxes_in(p, 0) {
                        if !tag_eq(&t, b"stsd") {
                            continue;
                        }
                        // stsd — fullbox: version/flags (4) + entry_count (4).
                        for (t, p) in boxes_in(p, 8) {
                            if !(tag_eq(&t, b"avc1") || tag_eq(&t, b"avc3")) {
                                continue;
                            }
                            // VisualSampleEntry: 78 байт заголовка, дальше боксы.
                            if p.len() < 78 + 8 {
                                continue;
                            }
                            for (t, p) in boxes_in(&p[78..], 0) {
                                if tag_eq(&t, b"avcC") {
                                    if let Some(c) = parse_avcc(p) {
                                        return Some(c);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Все payload боксов `mdat` верхнего уровня.
pub fn find_mdat_payloads(data: &[u8]) -> Vec<&[u8]> {
    boxes_in(data, 0)
        .into_iter()
        .filter(|(t, _)| tag_eq(t, b"mdat"))
        .map(|(_, p)| p)
        .collect()
}

/// Собрать Annex B первого IDR-кадра из length-prefixed `mdat`:
/// SPS/PPS из `avcC` + NAL до второго IDR (или до лимита).
/// Возвращает `None`, если IDR не найден.
pub fn length_prefixed_to_annexb(
    mdat_payloads: &[&[u8]],
    length_size: usize,
    sps_list: &[Vec<u8>],
    pps_list: &[Vec<u8>],
) -> Option<Vec<u8>> {
    if !(1..=4).contains(&length_size) {
        return None;
    }
    const START: &[u8] = &[0, 0, 0, 1];
    let mut out = Vec::new();
    for sps in sps_list {
        out.extend_from_slice(START);
        out.extend_from_slice(sps);
    }
    for pps in pps_list {
        out.extend_from_slice(START);
        out.extend_from_slice(pps);
    }
    let mut started = false;
    let mut nals = 0usize;
    'outer: for mdat in mdat_payloads {
        let mut off = 0usize;
        while off + length_size <= mdat.len() {
            let mut len = 0usize;
            for b in &mdat[off..off + length_size] {
                len = (len << 8) | (*b as usize);
            }
            off += length_size;
            if len == 0 || off + len > mdat.len() {
                break;
            }
            let nal = &mdat[off..off + len];
            off += len;
            if nal.is_empty() {
                continue;
            }
            let ty = nal_type(nal[0]);
            if ty == 5 && started {
                // Второй IDR — кадр собран целиком.
                break 'outer;
            }
            if ty == 5 {
                started = true;
            }
            if started {
                if out.len() + nal.len() + 4 > ANNEXB_LIMIT || nals >= MAX_NALS_PER_PICTURE {
                    break 'outer;
                }
                out.extend_from_slice(START);
                out.extend_from_slice(nal);
                nals += 1;
            }
            // До первого IDR пропускаем (P-кадры без опорного бесполезны).
        }
    }
    if !started {
        return None;
    }
    Some(out)
}

/// BT.601 (limited range) YUV420 → RGB. Плоскости плотные, как в `YuvFrame`.
pub fn yuv420_to_rgb(
    width: usize,
    height: usize,
    y: &[u8],
    u: &[u8],
    v: &[u8],
) -> Option<image::RgbImage> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return None;
    }
    if y.len() < width * height || u.len() < (width / 2) * (height / 2) || v.len() < (width / 2) * (height / 2) {
        return None;
    }
    let mut img = image::RgbImage::new(width as u32, height as u32);
    for row in 0..height {
        for col in 0..width {
            let yy = y[row * width + col] as i32;
            let uu = u[(row / 2) * (width / 2) + col / 2] as i32;
            let vv = v[(row / 2) * (width / 2) + col / 2] as i32;
            let c = yy - 16;
            let d = uu - 128;
            let e = vv - 128;
            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            img.put_pixel(col as u32, row as u32, image::Rgb([r, g, b]));
        }
    }
    Some(img)
}

/// Декодировать Annex B в первый кадр. `None` — битый поток (без паники).
pub fn decode_annexb_first_frame(annexb: &[u8]) -> Option<image::DynamicImage> {
    if annexb.len() < 16 {
        return None;
    }
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut dec = rusty_h264_decoder::Decoder::new();
        dec.decode(annexb)
    }));
    let frame = match res {
        Ok(Ok(Some(f))) => f,
        _ => return None,
    };
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let rgb = yuv420_to_rgb(frame.width, frame.height, &frame.y, &frame.u, &frame.v)?;
    Some(image::DynamicImage::ImageRgb8(rgb))
}

/// Извлечь первый кадр mp4/mov целиком из файла (avcC + mdat, лимит `SCAN_LIMIT`).
pub fn extract_frame(path: &Path) -> Option<image::DynamicImage> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    let n = meta.len().min(SCAN_LIMIT) as usize;
    let data = read_prefix(path, n)?;
    let cfg = find_avcc(&data)?;
    let mdat = find_mdat_payloads(&data);
    if mdat.is_empty() {
        return None;
    }
    let annexb = length_prefixed_to_annexb(&mdat, cfg.length_size, &cfg.sps, &cfg.pps)?;
    decode_annexb_first_frame(&annexb)
}

fn read_prefix(path: &Path, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; n];
    let mut read = 0usize;
    let mut f = f;
    while read < n {
        match f.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(k) => read += k,
            Err(_) => return None,
        }
    }
    buf.truncate(read);
    if buf.is_empty() {
        return None;
    }
    Some(buf)
}

/// Декодировать первый кадр и сохранить как JPEG-постер.
/// Возвращает путь постера или `None` (тогда вызывающий идёт дальше по цепочке).
pub fn poster_from_video(video: &Path, poster: &Path) -> Option<PathBuf> {
    let frame = extract_frame(video)?;
    if let Some(parent) = poster.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    // Масштабируем к портретному превью сторис, как ffmpeg-ветка.
    let small = frame.resize(270, 480, image::imageops::FilterType::Triangle);
    // Качество 80: постер для скоринга/превью, не архив.
    let f = std::fs::File::create(poster).ok()?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(f, 80);
    let rgb = small.to_rgb8();
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .ok()?;
    let meta = std::fs::metadata(poster).ok()?;
    if meta.len() < 1024 {
        return None;
    }
    Some(poster.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        v.extend_from_slice(tag);
        v.extend_from_slice(payload);
        v
    }

    fn sample_avcc() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let sps = vec![0x67, 0x42, 0x00, 0x1E, 0xA9, 0x18, 0x28, 0x2D];
        let pps = vec![0x68, 0xCE, 0x38, 0x80];
        let mut avcc = vec![0x01, 0x42, 0x00, 0x1E, 0xFF, 0xE1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&sps);
        avcc.push(1);
        avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&pps);
        (avcc, sps, pps)
    }

    #[test]
    fn avcc_roundtrip() {
        let (avcc, sps, pps) = sample_avcc();
        let cfg = parse_avcc(&avcc).expect("parse");
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.sps, vec![sps]);
        assert_eq!(cfg.pps, vec![pps]);
    }

    #[test]
    fn avcc_rejects_garbage() {
        assert!(parse_avcc(&[]).is_none());
        assert!(parse_avcc(&[0x02, 0, 0, 0, 0xFF, 0xE1, 0]).is_none()); // version != 1
        assert!(parse_avcc(&[0x01]).is_none());
        let (mut avcc, _, _) = sample_avcc();
        avcc.truncate(6);
        assert!(parse_avcc(&avcc).is_none());
    }

    fn sample_mp4(avcc: &[u8], mdat: &[u8]) -> Vec<u8> {
        // moov/trak/mdia/minf/stbl/stsd/avc1/avcC
        let mut avc1_inner = vec![0u8; 78];
        avc1_inner.extend_from_slice(&boxed(b"avcC", avcc));
        let avc1 = boxed(b"avc1", &avc1_inner);
        let mut stsd_payload = vec![0u8; 8]; // version/flags + entry_count
        stsd_payload.extend_from_slice(&avc1);
        let stsd = boxed(b"stsd", &stsd_payload);
        let stbl = boxed(b"stbl", &stsd);
        let minf = boxed(b"minf", &stbl);
        let mdia = boxed(b"mdia", &minf);
        let trak = boxed(b"trak", &mdia);
        let moov = boxed(b"moov", &trak);
        let mut file = boxed(b"ftyp", b"isom");
        file.extend_from_slice(&moov);
        file.extend_from_slice(&boxed(b"mdat", mdat));
        file
    }

    fn length_nal(payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn find_avcc_in_synthetic_mp4() {
        let (avcc, sps, pps) = sample_avcc();
        let mp4 = sample_mp4(&avcc, b"junk");
        let cfg = find_avcc(&mp4).expect("avcc found");
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.sps[0], sps);
        assert_eq!(cfg.pps[0], pps);
        // Без moov — None.
        assert!(find_avcc(b"not a file at all.............").is_none());
    }

    #[test]
    fn annexb_collects_first_idr_only() {
        let idr1 = vec![0x65, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x11, 0x22];
        let pframe = vec![0x41, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        let idr2 = vec![0x65, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let mut mdat = Vec::new();
        mdat.extend_from_slice(&length_nal(&[0x09, 0x10])); // AUD до IDR — пропуск
        mdat.extend_from_slice(&length_nal(&idr1));
        mdat.extend_from_slice(&length_nal(&pframe));
        mdat.extend_from_slice(&length_nal(&idr2)); // второй IDR — стоп
        let payloads = vec![mdat.as_slice()];
        let annexb = length_prefixed_to_annexb(&payloads, 4, &[vec![0x67]], &[vec![0x68]])
            .expect("annexb");
        // SPS/PPS + IDR1 + P-frame, без второго IDR.
        assert!(annexb.windows(idr1.len()).any(|w| w == idr1.as_slice()));
        assert!(annexb.windows(pframe.len()).any(|w| w == pframe.as_slice()));
        assert!(!annexb.windows(idr2.len()).any(|w| w == idr2.as_slice()));
        assert!(!annexb.windows(2).any(|w| w == [0x09, 0x10]));
    }

    #[test]
    fn annexb_none_without_idr() {
        let mut mdat = Vec::new();
        mdat.extend_from_slice(&length_nal(&[0x41, 0x01, 0x02, 0x03]));
        mdat.extend_from_slice(&length_nal(&[0x06, 0x05]));
        let payloads = vec![mdat.as_slice()];
        assert!(length_prefixed_to_annexb(&payloads, 4, &[vec![0x67]], &[vec![0x68]]).is_none());
        assert!(length_prefixed_to_annexb(&payloads, 9, &[vec![0x67]], &[vec![0x68]]).is_none());
    }

    #[test]
    fn yuv_black_and_white() {
        // Limited range: чёрный (16,128,128), белый (235,128,128).
        let black = yuv420_to_rgb(4, 4, &vec![16; 16], &vec![128; 4], &vec![128; 4]).unwrap();
        assert!(black.pixels().all(|p| p[0] <= 2 && p[1] <= 2 && p[2] <= 2));
        let white = yuv420_to_rgb(4, 4, &vec![235; 16], &vec![128; 4], &vec![128; 4]).unwrap();
        assert!(white.pixels().all(|p| p[0] >= 250 && p[1] >= 250 && p[2] >= 250));
        // Красный: Y~76-81, U~84-90, V~240.
        let red = yuv420_to_rgb(2, 2, &vec![81; 4], &vec![90; 1], &vec![240; 1]).unwrap();
        let p = red.get_pixel(0, 0);
        assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "got {p:?}");
        // Кривая геометрия/короткие плоскости — None.
        assert!(yuv420_to_rgb(3, 4, &vec![16; 12], &vec![128; 4], &vec![128; 4]).is_none());
        assert!(yuv420_to_rgb(4, 4, &vec![16; 4], &vec![128; 4], &vec![128; 4]).is_none());
    }

    #[test]
    fn decode_rejects_garbage_without_panic() {
        assert!(decode_annexb_first_frame(&[]).is_none());
        assert!(decode_annexb_first_frame(&[0u8; 8]).is_none());
        assert!(decode_annexb_first_frame(&vec![0xFF; 4096]).is_none());
        assert!(decode_annexb_first_frame(b"definitely not h264 annex b data....").is_none());
    }

    #[test]
    fn extract_frame_missing_file_is_none() {
        assert!(extract_frame(Path::new("/nope/no-video.mp4")).is_none());
        // Синтетический mp4 с фейковым IDR не декодируется, но и не паникует.
        let (avcc, _, _) = sample_avcc();
        let mut mdat = Vec::new();
        mdat.extend_from_slice(&length_nal(&[0x65, 0xAA, 0xBB, 0xCC]));
        let mp4 = sample_mp4(&avcc, &mdat);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("fake.mp4");
        std::fs::write(&f, &mp4).unwrap();
        assert!(extract_frame(&f).is_none());
    }
}
