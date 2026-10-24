//! Встроенный HEVC/H.265-декодер первого кадра (чистый Rust, без ffmpeg).
//!
//! Пара к `media/h264.rs`: Telegram отдаёт видео и в H.264 (`avc1`), и в
//! HEVC (`hvc1`/`hev1`) — второй случай раньше падал сразу на ffmpeg.
//! Теперь: `moov → hvcC (VPS/SPS/PPS)` + `mdat (length-prefixed NAL)` →
//! Annex B → `rusty_h265` (pure Rust, без C/FFI) → YUV → RGB → постер.
//!
//! Собираем первый IRAP-кадр (BLA/IDR/CRA, типы 16..=23): до него идут
//! только опорные P-кадры без опорного — бесполезны. На битых файлах
//! возвращаем `None` без паники (декодер под `catch_unwind`).

use std::path::{Path, PathBuf};

/// Сколько байт файла сканируем в поисках `moov`/`mdat`.
pub const SCAN_LIMIT: u64 = 24 * 1024 * 1024;
/// Лимит собранного Annex B для декодера.
pub const ANNEXB_LIMIT: usize = 8 * 1024 * 1024;
/// Максимум NAL-юнитов в одном кадре-поcтере.
pub const MAX_NALS_PER_PICTURE: usize = 2000;

/// Параметры потока из бокса `hvcC` (ISO/IEC 14496-15 §8.3.3.1.2).
#[derive(Debug, Clone)]
pub struct HevcConfig {
    /// Длина префикса NAL в `mdat` (обычно 4).
    pub length_size: usize,
    pub vps: Vec<Vec<u8>>,
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
}

/// Тип NAL-юнита HEVC: старшие 6 бит первого байта.
pub fn nal_type(byte0: u8) -> u8 {
    (byte0 >> 1) & 0x3F
}

/// Служебные (non-VCL) типы: VPS/SPS/PPS/PREFIX_SEI/SUFFIX_SEI/AUD.
pub fn is_parameter_or_sei(ty: u8) -> bool {
    matches!(ty, 32 | 33 | 34 | 39 | 40 | 35)
}

/// IRAP-кадр (точка случайного доступа): BLA/IDR/CRA 16..=23.
pub fn is_irap(ty: u8) -> bool {
    (16..=23).contains(&ty)
}

/// Распарсить payload бокса `hvcC` (без size+tag).
pub fn parse_hvcc(data: &[u8]) -> Option<HevcConfig> {
    if data.len() < 23 || data[0] != 1 {
        return None;
    }
    // Байт 21: lengthSizeMinusOne в младших 2 битах.
    let length_size = (data[21] & 0x03) as usize + 1;
    if !(1..=4).contains(&length_size) {
        return None;
    }
    let num_arrays = data[22] as usize;
    let mut off = 23usize;
    let (mut vps, mut sps, mut pps) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..num_arrays {
        if off + 3 > data.len() {
            return None;
        }
        let nal_type = data[off] & 0x3F;
        let num_nalus = u16::from_be_bytes([data[off + 1], data[off + 2]]) as usize;
        off += 3;
        for _ in 0..num_nalus {
            if off + 2 > data.len() {
                return None;
            }
            let len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
            off += 2;
            if len == 0 || off + len > data.len() {
                return None;
            }
            let nal = data[off..off + len].to_vec();
            off += len;
            match nal_type {
                32 => vps.push(nal),
                33 => sps.push(nal),
                34 => pps.push(nal),
                _ => {} // SEI и прочее для первого кадра не нужны
            }
        }
    }
    if vps.is_empty() || sps.is_empty() || pps.is_empty() {
        return None;
    }
    Some(HevcConfig {
        length_size,
        vps,
        sps,
        pps,
    })
}

/// Найти `hvcC` по пути moov→trak→mdia→minf→stbl→stsd→hev1/hvc1→hvcC.
/// Переиспользуем общий обходчик боксов из `h264` (тот же ISO BMFF).
pub fn find_hvcc(data: &[u8]) -> Option<HevcConfig> {
    use super::h264::boxes_in;
    for (t, p) in boxes_in(data, 0) {
        if &t == b"moov" {
            if let Some(c) = find_hvcc_in_moov(p) {
                return Some(c);
            }
        }
    }
    None
}

fn find_hvcc_in_moov(moov: &[u8]) -> Option<HevcConfig> {
    use super::h264::boxes_in;
    for (t, p) in boxes_in(moov, 0) {
        if &t != b"trak" {
            continue;
        }
        for (t, p) in boxes_in(p, 0) {
            if &t != b"mdia" {
                continue;
            }
            for (t, p) in boxes_in(p, 0) {
                if &t != b"minf" {
                    continue;
                }
                for (t, p) in boxes_in(p, 0) {
                    if &t != b"stbl" {
                        continue;
                    }
                    for (t, p) in boxes_in(p, 0) {
                        if &t != b"stsd" {
                            continue;
                        }
                        for (t, p) in boxes_in(p, 8) {
                            if &t != b"hev1" && &t != b"hvc1" {
                                continue;
                            }
                            if p.len() < 78 + 8 {
                                continue;
                            }
                            for (t, p) in boxes_in(&p[78..], 0) {
                                if &t == b"hvcC" {
                                    if let Some(c) = parse_hvcc(p) {
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
    super::h264::boxes_in(data, 0)
        .into_iter()
        .filter(|(t, _)| t == b"mdat")
        .map(|(_, p)| p)
        .collect()
}

/// Собрать Annex B первого IRAP-кадра из length-prefixed `mdat`:
/// VPS/SPS/PPS из `hvcC` + NAL до следующего IRAP (или до лимита).
/// `None`, если IRAP не найден.
pub fn length_prefixed_to_annexb(
    mdat_payloads: &[&[u8]],
    length_size: usize,
    vps_list: &[Vec<u8>],
    sps_list: &[Vec<u8>],
    pps_list: &[Vec<u8>],
) -> Option<Vec<u8>> {
    if !(1..=4).contains(&length_size) {
        return None;
    }
    const START: &[u8] = &[0, 0, 0, 1];
    let mut out = Vec::new();
    for set in vps_list.iter().chain(sps_list).chain(pps_list) {
        out.extend_from_slice(START);
        out.extend_from_slice(set);
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
            if len < 2 || off + len > mdat.len() {
                break;
            }
            let nal = &mdat[off..off + len];
            off += len;
            let ty = nal_type(nal[0]);
            if is_irap(ty) && started {
                // Второй IRAP — кадр собран целиком.
                break 'outer;
            }
            if is_irap(ty) {
                started = true;
            }
            if started || is_parameter_or_sei(ty) {
                if out.len() + nal.len() + 4 > ANNEXB_LIMIT || nals >= MAX_NALS_PER_PICTURE {
                    break 'outer;
                }
                // Параметровые NAL до первого IRAP тоже кладём (декодеру нужны).
                out.extend_from_slice(START);
                out.extend_from_slice(nal);
                nals += 1;
            }
        }
    }
    if !started {
        return None;
    }
    Some(out)
}

/// Кадр rusty_h265 → RGB. Поддерживаем 4:2:0, 8 бит и 10 бит (сдвиг).
/// Возвращает `None` для экзотики (4:2:2/4:4:4, монохром, нечётная геометрия).
pub fn hevc_frame_to_rgb(frame: &rusty_h265::Frame) -> Option<image::RgbImage> {
    let pic = &frame.picture;
    if pic.chroma_format_idc != 1 {
        return None;
    }
    let (cx, cy, cw, ch) = pic.crop;
    let (w, h) = (frame.width, frame.height);
    if cw != w || ch != h || w == 0 || h == 0 || !w.is_multiple_of(2) || !h.is_multiple_of(2) {
        return None;
    }
    let shift = match pic.bit_depth_luma {
        8 => 0u32,
        9..=16 => (pic.bit_depth_luma - 8) as u32,
        _ => return None,
    };
    // Плоскости u16, stride == ширине плоскости (Plane::new(w,h) плотный).
    let narrow = |plane: &rusty_h265::Plane,
                  x0: usize,
                  y0: usize,
                  pw: usize,
                  ph: usize|
     -> Option<Vec<u8>> {
        let mut v = Vec::with_capacity(pw * ph);
        for y in y0..y0 + ph {
            let row = plane.row(y);
            if row.len() < x0 + pw {
                return None;
            }
            for &s in &row[x0..x0 + pw] {
                v.push((s >> shift).min(255) as u8);
            }
        }
        Some(v)
    };
    let y = narrow(&pic.planes[0], cx, cy, w, h)?;
    let u = narrow(&pic.planes[1], cx / 2, cy / 2, w / 2, h / 2)?;
    let v = narrow(&pic.planes[2], cx / 2, cy / 2, w / 2, h / 2)?;
    super::h264::yuv420_to_rgb(w, h, &y, &u, &v)
}

/// Декодировать Annex B в первый кадр. `None` — битый поток (без паники).
pub fn decode_annexb_first_frame(annexb: &[u8]) -> Option<image::DynamicImage> {
    if annexb.len() < 16 {
        return None;
    }
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut dec = rusty_h265::Decoder::new();
        if dec.push_annexb(annexb, None).is_err() {
            // Частичный поток: забираем то, что успело декодироваться.
        }
        // Error::Again — кадры кончились, забираем последний.
        let mut last = None;
        while let Ok(f) = dec.next_frame() {
            last = Some(f);
        }
        last
    }));
    let frame = match res {
        Ok(Some(f)) => f,
        _ => return None,
    };
    let rgb = hevc_frame_to_rgb(&frame)?;
    Some(image::DynamicImage::ImageRgb8(rgb))
}

/// Извлечь первый кадр HEVC mp4/mov целиком (hvcC + mdat, лимит `SCAN_LIMIT`).
pub fn extract_frame(path: &Path) -> Option<image::DynamicImage> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    let n = meta.len().min(SCAN_LIMIT) as usize;
    let data = read_prefix(path, n)?;
    let cfg = find_hvcc(&data)?;
    let mdat = find_mdat_payloads(&data);
    if mdat.is_empty() {
        return None;
    }
    let annexb = length_prefixed_to_annexb(&mdat, cfg.length_size, &cfg.vps, &cfg.sps, &cfg.pps)?;
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
pub fn poster_from_video(video: &Path, poster: &Path) -> Option<PathBuf> {
    let frame = extract_frame(video)?;
    if let Some(parent) = poster.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let small = frame.resize(270, 480, image::imageops::FilterType::Triangle);
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

    /// Минимальный валидный hvcC: version + профиль/уровень + lengthSize=4
    /// + по одному массиву VPS/SPS/PPS.
    fn sample_hvcc() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let vps = vec![0x40, 0x01, 0x0C, 0x01, 0xFF, 0xFF, 0x01, 0x60];
        let sps = vec![0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03];
        let pps = vec![0x44, 0x01, 0xC0, 0x73, 0xC1, 0x2C, 0x10, 0x00];
        let mut h = vec![
            0x01, // configurationVersion
            0x01, // profile_space(0)/tier(0)/profile_idc(1)
            0x60, 0x00, 0x00, 0x00, // compatibility
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // constraint flags
            0x78, // level_idc
            0xF0, 0x00, // min_spatial_segmentation + reserved
            0xFC, // parallelismType
            0xFD, // chromaFormat
            0xF8, // bitDepthLumaMinus8
            0xF8, // bitDepthChromaMinus8
            0x00, 0x00, // avgFrameRate
            0xFF, // constantFrameRate/numTemporalLayers/nested + lengthSizeMinusOne=3
            0x03, // numOfArrays
        ];
        for (ty, nal) in [(32u8, &vps), (33, &sps), (34, &pps)] {
            h.push(0x80 | ty); // array_completeness + NAL type
            h.extend_from_slice(&1u16.to_be_bytes());
            h.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            h.extend_from_slice(nal);
        }
        assert_eq!(
            &h[..23],
            &[
                0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0xF0,
                0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0x00, 0x00, 0xFF, 0x03,
            ][..]
        ); // lengthSizeMinusOne = 3 → 4 байта, numOfArrays = 3
        (h, vps, sps, pps)
    }

    #[test]
    fn hvcc_roundtrip() {
        let (hvcc, vps, sps, pps) = sample_hvcc();
        let cfg = parse_hvcc(&hvcc).expect("parse");
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.vps, vec![vps]);
        assert_eq!(cfg.sps, vec![sps]);
        assert_eq!(cfg.pps, vec![pps]);
    }

    #[test]
    fn hvcc_rejects_garbage() {
        assert!(parse_hvcc(&[]).is_none());
        assert!(parse_hvcc(&[0x02; 30]).is_none()); // version != 1
        assert!(parse_hvcc(&[0x01; 10]).is_none()); // коротко
        let (mut h, _, _, _) = sample_hvcc();
        h.truncate(22);
        assert!(parse_hvcc(&h).is_none());
        // Без PPS — неполный набор.
        let (mut h2, _, _, _) = sample_hvcc();
        h2[22] = 2;
        h2.truncate(h2.len() - 11);
        assert!(parse_hvcc(&h2).is_none());
    }

    fn sample_mp4(hvcc: &[u8], mdat: &[u8]) -> Vec<u8> {
        let mut hvc1_inner = vec![0u8; 78];
        hvc1_inner.extend_from_slice(&boxed(b"hvcC", hvcc));
        let hvc1 = boxed(b"hvc1", &hvc1_inner);
        let mut stsd_payload = vec![0u8; 8];
        stsd_payload.extend_from_slice(&hvc1);
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
    fn find_hvcc_in_synthetic_mp4() {
        let (hvcc, vps, sps, pps) = sample_hvcc();
        let mp4 = sample_mp4(&hvcc, b"junk");
        let cfg = find_hvcc(&mp4).expect("hvcc found");
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.vps[0], vps);
        assert_eq!(cfg.sps[0], sps);
        assert_eq!(cfg.pps[0], pps);
        // H.264-файл hvcC не содержит.
        assert!(find_hvcc(b"avc1....not hevc................").is_none());
    }

    #[test]
    fn nal_types() {
        assert_eq!(nal_type(0x26), 19); // IDR_W_RADL
        assert_eq!(nal_type(0x28), 20); // IDR_N_LP
        assert_eq!(nal_type(0x2A), 21); // CRA_NUT
        assert!(is_irap(19) && is_irap(21) && !is_irap(1) && !is_irap(32));
        assert!(is_parameter_or_sei(33) && !is_parameter_or_sei(19));
    }

    #[test]
    fn annexb_collects_first_irap_only() {
        let trail = vec![0x02, 0x01, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]; // TRAIL_N
        let idr = vec![0x26, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]; // IDR_W_RADL
        let idr2 = vec![0x28, 0x01, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11]; // IDR_N_LP
        let mut mdat = Vec::new();
        mdat.extend_from_slice(&length_nal(&trail)); // до IRAP — пропуск
        mdat.extend_from_slice(&length_nal(&idr));
        mdat.extend_from_slice(&length_nal(&trail));
        mdat.extend_from_slice(&length_nal(&idr2)); // второй IRAP — стоп
        let payloads = vec![mdat.as_slice()];
        let annexb =
            length_prefixed_to_annexb(&payloads, 4, &[vec![0x40]], &[vec![0x42]], &[vec![0x44]])
                .expect("annexb");
        assert!(annexb.windows(idr.len()).any(|w| w == idr.as_slice()));
        assert!(!annexb.windows(idr2.len()).any(|w| w == idr2.as_slice()));
        // TRAIL до IRAP пропущен, после — взят.
        let trail_hits = annexb
            .windows(trail.len())
            .filter(|w| *w == trail.as_slice())
            .count();
        assert_eq!(trail_hits, 1);
    }

    #[test]
    fn annexb_none_without_irap() {
        let mut mdat = Vec::new();
        mdat.extend_from_slice(&length_nal(&[0x02, 0x01, 0x02, 0x03]));
        let payloads = vec![mdat.as_slice()];
        assert!(length_prefixed_to_annexb(
            &payloads,
            4,
            &[vec![0x40]],
            &[vec![0x42]],
            &[vec![0x44]]
        )
        .is_none());
    }

    #[test]
    fn decode_rejects_garbage_without_panic() {
        assert!(decode_annexb_first_frame(&[]).is_none());
        assert!(decode_annexb_first_frame(&[0u8; 8]).is_none());
        assert!(decode_annexb_first_frame(&vec![0xFF; 4096]).is_none());
        assert!(decode_annexb_first_frame(b"definitely not hevc annex b....").is_none());
    }

    #[test]
    fn extract_frame_missing_or_fake_is_none() {
        assert!(extract_frame(Path::new("/nope/no-video.mp4")).is_none());
        let (hvcc, _, _, _) = sample_hvcc();
        let mut mdat = Vec::new();
        mdat.extend_from_slice(&length_nal(&[0x26, 0x01, 0xAA, 0xBB]));
        let mp4 = sample_mp4(&hvcc, &mdat);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("fake-hevc.mp4");
        std::fs::write(&f, &mp4).unwrap();
        assert!(extract_frame(&f).is_none());
    }
}
