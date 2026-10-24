//! Встроенный AV1-декодер первого кадра (чистый Rust, без ffmpeg).
//!
//! Пара к `media/h264.rs` и `media/hevc.rs`: Telegram отдаёт видео в
//! H.264 (`avc1`), HEVC (`hvc1`/`hev1`) и AV1 (`av01`, WebM/MKV тоже).
//! Третий случай раньше падал сразу на внешний `ffmpeg`.
//! Теперь: `moov → av01 → av1C (configOBUs)` + `mdat (OBU-поток)` →
//! `rusty_av1d` (форк rav1d — dav1d, портированный на Rust, pure-Rust
//! build path без asm/nasm) → YUV → RGB → постер.
//!
//! Для WebM/MKV — минимальный EBML-парсер (чистый Rust, без зависимостей):
//! `CodecPrivate (configOBUs)` + `Cluster/SimpleBlock|BlockGroup/Block`
//! (кадры = Temporal Unit'ы, OBU уже самоделённые) → тот же декодер.
//!
//! Всё тяжёлое — за чистыми функциями с тестами; на битых файлах
//! возвращаем `None` без паники (декодер дополнительно обёрнут
//! в `catch_unwind`, т.к. вход — недоверенные байты из сети).

use std::path::{Path, PathBuf};

/// Сколько байт файла сканируем в поисках `moov`/`mdat`/EBML-кластеров
/// (moov у faststart-файлов Telegram — в начале; лимит защищает от чтения
/// 1.5ГБ в RAM).
pub const SCAN_LIMIT: u64 = 24 * 1024 * 1024;
/// Лимит собранного OBU-потока для декодера (первый кадр + configOBUs).
pub const OBU_LIMIT: usize = 8 * 1024 * 1024;
/// Максимум кадров WebM/блоков, склеиваемых в один поток (первый показной
/// кадр обычно в первых единицах; защита от склейки всего фильма).
pub const MAX_WEBM_FRAMES: usize = 32;
/// Максимум OBU-юнитов при эвристическом разборе потока.
pub const MAX_OBUS: usize = 4000;

/// Параметры потока из бокса `av1C` (AV1-ISOBMFF §2.2).
#[derive(Debug, Clone)]
pub struct Av1Config {
    /// configOBUs — обычно sequence header OBU, кладём в начало потока.
    pub config_obus: Vec<u8>,
    pub seq_profile: u8,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub monochrome: bool,
}

/// Распарсить payload бокса `av1C` (без size+tag).
/// Минимум — 4 байта заголовка; configOBUs могут быть пустыми
/// (тогда sequence header ждём из самого потока).
pub fn parse_av1c(data: &[u8]) -> Option<Av1Config> {
    if data.len() < 4 {
        return None;
    }
    // marker (1 бит) должен быть 1, version (7 бит) — 1.
    if data[0] != 0x81 {
        return None;
    }
    let seq_profile = (data[1] >> 5) & 0x07;
    if seq_profile > 2 {
        return None;
    }
    let b2 = data[2];
    let high_bitdepth = b2 & 0x40 != 0;
    let twelve_bit = b2 & 0x20 != 0;
    let monochrome = b2 & 0x10 != 0;
    Some(Av1Config {
        config_obus: data[4..].to_vec(),
        seq_profile,
        high_bitdepth,
        twelve_bit,
        monochrome,
    })
}

/// Тип OBU-юнита: биты 3..=6 заголовка.
pub fn obu_type(byte0: u8) -> u8 {
    (byte0 >> 3) & 0x0F
}

/// Валидный заголовок OBU для эвристик: forbidden-бит 0, тип из известных.
pub fn obu_header_valid(byte0: u8) -> bool {
    if byte0 & 0x80 != 0 {
        return false;
    }
    matches!(obu_type(byte0), 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 15)
}

/// OBU имеет поле размера (кроме temporal delimiter).
pub fn obu_has_size(byte0: u8) -> bool {
    if obu_type(byte0) == 2 {
        return false;
    }
    byte0 & 0x02 != 0
}

/// Читать unsigned LEB128. Возвращает (значение, длина в байтах).
pub fn read_leb128(data: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    for (i, &b) in data.iter().enumerate().take(8) {
        let chunk = (b & 0x7F) as u64;
        value |= chunk << (7 * i);
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// Пройти OBU-поток, вернуть число юнитов и потреблённые байты.
/// Останавливается на первом невалидном юните.
pub fn parse_obu_stream(data: &[u8]) -> (usize, usize) {
    let mut off = 0usize;
    let mut count = 0usize;
    while off < data.len() && count < MAX_OBUS {
        let h = data[off];
        if !obu_header_valid(h) {
            break;
        }
        let mut hdr_len = 1usize;
        if h & 0x04 != 0 {
            // obu_extension_flag — ещё байт.
            hdr_len += 1;
        }
        if obu_type(h) == 2 {
            // Temporal delimiter: только заголовок.
            off += hdr_len;
            count += 1;
            continue;
        }
        if !obu_has_size(h) {
            // Без размера границы не восстановить — только TD так хранят.
            break;
        }
        let Some((size, leb_len)) = read_leb128(&data[off + hdr_len..]) else {
            break;
        };
        let size = size as usize;
        let total = hdr_len + leb_len + size;
        if off + total > data.len() {
            break;
        }
        off += total;
        count += 1;
    }
    (count, off)
}

/// Найти `av1C` по пути moov→trak→mdia→minf→stbl→stsd→av01→av1C.
/// Переиспользуем общий обходчик боксов из `h264` (тот же ISO BMFF).
pub fn find_av1c(data: &[u8]) -> Option<Av1Config> {
    use super::h264::boxes_in;
    for (t, p) in boxes_in(data, 0) {
        if &t == b"moov" {
            if let Some(c) = find_av1c_in_moov(p) {
                return Some(c);
            }
        }
    }
    None
}

fn find_av1c_in_moov(moov: &[u8]) -> Option<Av1Config> {
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
                            if &t != b"av01" {
                                continue;
                            }
                            // VisualSampleEntry: 78 байт заголовка, дальше боксы.
                            if p.len() < 78 + 8 {
                                continue;
                            }
                            for (t, p) in boxes_in(&p[78..], 0) {
                                if &t == b"av1C" {
                                    if let Some(c) = parse_av1c(p) {
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

/// Нормализовать сэмпл MP4: некоторые муксеры кладут OBU-юниты с
/// 4-байтным BE-префиксом длины (по аналогии с avcC). Если буфер целиком
/// разбирается как цепочка (len + валидный OBU) — префиксы срезаем,
/// иначе считаем потоком сырых OBU (self-delimited через LEB128).
pub fn normalize_mp4_sample(sample: &[u8]) -> Vec<u8> {
    if let Some(stripped) = try_strip_lengths(sample) {
        return stripped;
    }
    sample.to_vec()
}

fn try_strip_lengths(sample: &[u8]) -> Option<Vec<u8>> {
    if sample.len() < 6 {
        return None;
    }
    let mut off = 0usize;
    let mut out = Vec::new();
    let mut units = 0usize;
    while off + 4 <= sample.len() {
        let len = u32::from_be_bytes([
            sample[off],
            sample[off + 1],
            sample[off + 2],
            sample[off + 3],
        ]) as usize;
        if len == 0 || len > sample.len() - off - 4 || len < 2 {
            return None;
        }
        if !obu_header_valid(sample[off + 4]) {
            return None;
        }
        out.extend_from_slice(&sample[off + 4..off + 4 + len]);
        off += 4 + len;
        units += 1;
        if units > MAX_OBUS {
            return None;
        }
    }
    if off != sample.len() || units == 0 {
        return None;
    }
    Some(out)
}

/// Собрать OBU-поток для декодера: configOBUs из `av1C` + нормализованные
/// `mdat`-сэмплы (до лимита). `None`, если собирать не из чего.
pub fn obu_stream_from_mdat(mdat_payloads: &[&[u8]], config_obus: &[u8]) -> Option<Vec<u8>> {
    if mdat_payloads.is_empty() && config_obus.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(config_obus.len() + 4096);
    if !config_obus.is_empty() {
        // configOBUs — уже OBU-поток (sequence header с размерами).
        let (n, consumed) = parse_obu_stream(config_obus);
        if n == 0 {
            // Не похож на OBU — всё равно кладём как есть, декодер разберётся.
            out.extend_from_slice(config_obus);
        } else {
            out.extend_from_slice(&config_obus[..consumed]);
            // Хвост после валидных OBU игнорируем (мусор пакера).
        }
    }
    let mut any = !config_obus.is_empty();
    for mdat in mdat_payloads {
        if out.len() >= OBU_LIMIT {
            break;
        }
        // Пробуем целиком: сырой OBU-поток предпочтителен.
        let (n, consumed) = parse_obu_stream(mdat);
        if n > 0 && consumed * 2 >= mdat.len() {
            let take = consumed.min(OBU_LIMIT - out.len());
            out.extend_from_slice(&mdat[..take]);
            any = true;
        } else {
            let norm = normalize_mp4_sample(mdat);
            if norm.len() != mdat.len() || parse_obu_stream(&norm).0 > 0 {
                let take = norm.len().min(OBU_LIMIT - out.len());
                out.extend_from_slice(&norm[..take]);
                any = true;
            } else if mdat.len() + out.len() <= OBU_LIMIT {
                // Не распознали, но отдаём как есть — вдруг декодер съест.
                out.extend_from_slice(mdat);
                any = true;
            }
        }
    }
    if !any || out.len() < 8 {
        return None;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// WebM/MKV: минимальный EBML-парсер (только нужное для AV1-кадров).
// ---------------------------------------------------------------------------

/// EBML-заголовок файла WebM/MKV.
pub fn looks_like_ebml(data: &[u8]) -> bool {
    data.len() >= 4 && data[0] == 0x1A && data[1] == 0x45 && data[2] == 0xDF && data[3] == 0xA3
}

/// Длина ID по первому байту (маркер длины входит в ID).
fn ebml_id_len(first: u8) -> Option<usize> {
    for i in 0..8 {
        if first & (0x80 >> i) != 0 {
            return Some(i + 1);
        }
    }
    None
}

/// Прочитать EBML ID (значение с маркером) + длина.
fn ebml_read_id(data: &[u8], off: usize) -> Option<(u64, usize)> {
    let len = ebml_id_len(*data.get(off)?)?;
    if off + len > data.len() {
        return None;
    }
    let mut v: u64 = 0;
    for b in &data[off..off + len] {
        v = (v << 8) | (*b as u64);
    }
    Some((v, len))
}

/// Прочитать VINT-размер (маркер сброшен). `None` = усечено.
/// Возвращает (размер, длина); размер `u64::MAX` = unknown-size.
fn ebml_read_vint(data: &[u8], off: usize) -> Option<(u64, usize)> {
    let len = ebml_id_len(*data.get(off)?)?;
    if off + len > data.len() {
        return None;
    }
    let mut v: u64 = (*data.get(off)? & ((0xFFu16 >> len) as u8)) as u64;
    for b in &data[off + 1..off + len] {
        v = (v << 8) | (*b as u64);
    }
    // Все биты значения = 1 → unknown-size (до конца родителя).
    let all_ones = (1u64 << (7 * len)) - 1;
    if v == all_ones {
        return Some((u64::MAX, len));
    }
    Some((v, len))
}

/// Прочитать VINT-значение трека (маркер сброшен, unknown-size невозможен).
fn ebml_read_track(data: &[u8], off: usize) -> Option<(u64, usize)> {
    ebml_read_vint(data, off).filter(|(v, _)| *v != u64::MAX)
}

const EBML_SEGMENT: u64 = 0x18538067;
const EBML_TRACKS: u64 = 0x1654AE6B;
const EBML_CODEC_PRIVATE: u64 = 0x63A2;
const EBML_CLUSTER: u64 = 0x1F43B675;
const EBML_SIMPLE_BLOCK: u64 = 0xA3;
const EBML_BLOCK_GROUP: u64 = 0xA0;
const EBML_BLOCK: u64 = 0xA1;

/// Извлечь из EBML-префикса (configOBUs, кадры): первый найденный
/// `CodecPrivate` + payload'ы `SimpleBlock`/`Block` без лейсинга.
/// Усечённый хвост парсится до первого битого элемента.
pub fn webm_frames(data: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut codec_private = Vec::new();
    let mut frames: Vec<Vec<u8>> = Vec::new();
    // Обход верхнего уровня → Segment → дети.
    let mut off = 0usize;
    while off < data.len() && frames.len() < MAX_WEBM_FRAMES {
        let Some((id, id_len)) = ebml_read_id(data, off) else {
            break;
        };
        let Some((size, size_len)) = ebml_read_vint(data, off + id_len) else {
            break;
        };
        let hdr = id_len + size_len;
        let end = if size == u64::MAX {
            data.len()
        } else {
            (off + hdr + size as usize).min(data.len())
        };
        if off + hdr > data.len() {
            break;
        }
        let payload = &data[off + hdr..end];
        if id == EBML_SEGMENT {
            walk_segment(payload, &mut codec_private, &mut frames);
            if frames.len() >= MAX_WEBM_FRAMES {
                break;
            }
        }
        if end <= off {
            break;
        }
        off = end;
    }
    (codec_private, frames)
}

fn walk_segment(seg: &[u8], codec_private: &mut Vec<u8>, frames: &mut Vec<Vec<u8>>) {
    let mut off = 0usize;
    while off < seg.len() && frames.len() < MAX_WEBM_FRAMES {
        let Some((id, id_len)) = ebml_read_id(seg, off) else {
            break;
        };
        let Some((size, size_len)) = ebml_read_vint(seg, off + id_len) else {
            break;
        };
        let hdr = id_len + size_len;
        if off + hdr > seg.len() {
            break;
        }
        let end = if size == u64::MAX {
            seg.len()
        } else {
            (off + hdr + size as usize).min(seg.len())
        };
        let payload = &seg[off + hdr..end];
        match id {
            EBML_TRACKS => {
                if codec_private.is_empty() {
                    if let Some(cp) = find_codec_private(payload) {
                        *codec_private = cp;
                    }
                }
            }
            EBML_CLUSTER => walk_cluster(payload, frames),
            _ => {}
        }
        if end <= off {
            break;
        }
        off = end;
    }
}

/// Рекурсивный поиск первого `CodecPrivate` (данные — как есть, для AV1
/// это configOBUs = sequence header).
fn find_codec_private(data: &[u8]) -> Option<Vec<u8>> {
    let mut off = 0usize;
    while off < data.len() {
        let (id, id_len) = ebml_read_id(data, off)?;
        let (size, size_len) = ebml_read_vint(data, off + id_len)?;
        let hdr = id_len + size_len;
        if off + hdr > data.len() {
            return None;
        }
        let end = if size == u64::MAX {
            data.len()
        } else {
            (off + hdr + size as usize).min(data.len())
        };
        let payload = &data[off + hdr..end];
        if id == EBML_CODEC_PRIVATE && size != u64::MAX && !payload.is_empty() {
            return Some(payload.to_vec());
        }
        // Заглядываем внутрь контейнеров (Tracks/TrackEntry и т.д.).
        if id == EBML_TRACKS || (0xAE..=0xAF).contains(&id) || id > 0xFF {
            if let Some(found) = find_codec_private(payload) {
                return Some(found);
            }
        }
        if end <= off {
            break;
        }
        off = end;
    }
    None
}

fn walk_cluster(cluster: &[u8], frames: &mut Vec<Vec<u8>>) {
    let mut off = 0usize;
    while off < cluster.len() && frames.len() < MAX_WEBM_FRAMES {
        let Some((id, id_len)) = ebml_read_id(cluster, off) else {
            break;
        };
        let Some((size, size_len)) = ebml_read_vint(cluster, off + id_len) else {
            break;
        };
        let hdr = id_len + size_len;
        if off + hdr > cluster.len() {
            break;
        }
        let end = if size == u64::MAX {
            cluster.len()
        } else {
            (off + hdr + size as usize).min(cluster.len())
        };
        let payload = &cluster[off + hdr..end];
        match id {
            EBML_SIMPLE_BLOCK => {
                if let Some(frame) = parse_simple_block(payload) {
                    frames.push(frame);
                }
            }
            EBML_BLOCK_GROUP => {
                if let Some(frame) = parse_block_group(payload) {
                    frames.push(frame);
                }
            }
            _ => {}
        }
        if end <= off {
            break;
        }
        off = end;
    }
}

/// SimpleBlock: track vint + int16 timecode + flags + кадр (без лейсинга).
fn parse_simple_block(payload: &[u8]) -> Option<Vec<u8>> {
    let (_, tlen) = ebml_read_track(payload, 0)?;
    if payload.len() < tlen + 3 {
        return None;
    }
    let flags = payload[tlen + 2];
    // Биты 1-2 — лейсинг: 0 = нет. Лейсинг не разбираем.
    if flags & 0x06 != 0 {
        return None;
    }
    let frame = &payload[tlen + 3..];
    if frame.is_empty() {
        return None;
    }
    Some(frame.to_vec())
}

/// BlockGroup: ищем вложенный Block, заголовок как у SimpleBlock.
fn parse_block_group(payload: &[u8]) -> Option<Vec<u8>> {
    let mut off = 0usize;
    while off < payload.len() {
        let (id, id_len) = ebml_read_id(payload, off)?;
        let (size, size_len) = ebml_read_vint(payload, off + id_len)?;
        let hdr = id_len + size_len;
        if off + hdr > payload.len() {
            return None;
        }
        let end = if size == u64::MAX {
            payload.len()
        } else {
            (off + hdr + size as usize).min(payload.len())
        };
        if id == EBML_BLOCK {
            let inner = &payload[off + hdr..end];
            let (_, tlen) = ebml_read_track(inner, 0)?;
            if inner.len() < tlen + 3 {
                return None;
            }
            // У Block после timecode идёт 1 байт flags (только лейсинг).
            if inner[tlen + 2] & 0x06 != 0 {
                return None;
            }
            let frame = &inner[tlen + 3..];
            if frame.is_empty() {
                return None;
            }
            return Some(frame.to_vec());
        }
        if end <= off {
            break;
        }
        off = end;
    }
    None
}

/// Собрать OBU-поток из WebM/MKV: CodecPrivate + кадры до лимита.
pub fn obu_stream_from_webm(codec_private: &[u8], frames: &[Vec<u8>]) -> Option<Vec<u8>> {
    if codec_private.is_empty() && frames.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(codec_private);
    for f in frames {
        if out.len() + f.len() > OBU_LIMIT {
            break;
        }
        out.extend_from_slice(f);
    }
    if out.len() < 8 {
        return None;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Декодер.
// ---------------------------------------------------------------------------

/// Один пиксель BT.601: limited (`tv`) или full range.
pub fn yuv_to_rgb_pixel(y: u8, u: u8, v: u8, full_range: bool) -> [u8; 3] {
    if full_range {
        let yf = y as f32;
        let uf = u as f32 - 128.0;
        let vf = v as f32 - 128.0;
        [
            (yf + 1.402 * vf).round().clamp(0.0, 255.0) as u8,
            (yf - 0.344136 * uf - 0.714136 * vf)
                .round()
                .clamp(0.0, 255.0) as u8,
            (yf + 1.772 * uf).round().clamp(0.0, 255.0) as u8,
        ]
    } else {
        let c = y as i32 - 16;
        let d = u as i32 - 128;
        let e = v as i32 - 128;
        [
            ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8,
            ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8,
            ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8,
        ]
    }
}

/// Кадр `rusty_av1d` → RGB. Поддерживаем 4:0:0/4:2:0/4:2:2/4:4:4,
/// 8/10/12 бит (старшие биты), limited + full range.
/// `None` для экзотики.
pub fn picture_to_rgb(pic: &rusty_av1d::Picture) -> Option<image::RgbImage> {
    use rusty_av1d::{pixel::YUVRange, PlanarImageComponent as C};
    let (w, h) = (pic.width() as usize, pic.height() as usize);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return None;
    }
    let (ss_x, ss_y, has_chroma) = match pic.pixel_layout() {
        rusty_av1d::PixelLayout::I400 => (0usize, 0usize, false),
        rusty_av1d::PixelLayout::I420 => (1, 1, true),
        rusty_av1d::PixelLayout::I422 => (1, 0, true),
        rusty_av1d::PixelLayout::I444 => (0, 0, true),
    };
    let (cw, ch) = ((w + ss_x) >> ss_x, (h + ss_y) >> ss_y);
    let full_range = matches!(pic.color_range(), YUVRange::Full);
    let storage16 = pic.bit_depth() == 16;
    let shift = pic
        .bits_per_component()
        .map(|b| b.0.saturating_sub(8) as u32)
        .unwrap_or(0);

    fn read_plane(
        pic: &rusty_av1d::Picture,
        comp: C,
        pw: usize,
        ph: usize,
        storage16: bool,
        shift: u32,
    ) -> Option<Vec<u8>> {
        let stride = pic.stride(comp) as usize;
        if stride < pw || stride > 16384 {
            return None;
        }
        let bytes = pic.plane(comp);
        if !storage16 {
            if bytes.len() < stride * ph {
                return None;
            }
            let mut v = Vec::with_capacity(pw * ph);
            for row in 0..ph {
                v.extend_from_slice(&bytes[row * stride..row * stride + pw]);
            }
            Some(v)
        } else {
            if bytes.len() < stride * ph * 2 {
                return None;
            }
            let mut v = Vec::with_capacity(pw * ph);
            for row in 0..ph {
                let base = row * stride * 2;
                for col in 0..pw {
                    let lo = bytes[base + col * 2] as u16;
                    let hi = bytes[base + col * 2 + 1] as u16;
                    v.push(((lo | (hi << 8)) >> shift).min(255) as u8);
                }
            }
            Some(v)
        }
    }

    let y = read_plane(pic, C::Y, w, h, storage16, shift)?;
    let (u, v) = if has_chroma {
        (
            read_plane(pic, C::U, cw, ch, storage16, shift)?,
            read_plane(pic, C::V, cw, ch, storage16, shift)?,
        )
    } else {
        (vec![128u8; cw * ch], vec![128u8; cw * ch])
    };
    let mut img = image::RgbImage::new(w as u32, h as u32);
    for row in 0..h {
        for col in 0..w {
            let yy = y[row * w + col];
            let (uu, vv) = if has_chroma {
                (
                    u[(row >> ss_y) * cw + (col >> ss_x)],
                    v[(row >> ss_y) * cw + (col >> ss_x)],
                )
            } else {
                (128, 128)
            };
            let [r, g, b] = yuv_to_rgb_pixel(yy, uu, vv, full_range);
            img.put_pixel(col as u32, row as u32, image::Rgb([r, g, b]));
        }
    }
    Some(img)
}

/// Декодировать OBU-поток в первый показной кадр. `None` — мусор (без паники).
pub fn decode_obu_first_frame(obu: &[u8]) -> Option<image::DynamicImage> {
    if obu.len() < 8 {
        return None;
    }
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut dec = rusty_av1d::Decoder::new().ok()?;
        // Один temporal unit за раз: сначала пробуем весь поток целиком.
        let data: Box<[u8]> = obu.to_vec().into_boxed_slice();
        let _ = dec.send_data(data, None, None, None);
        for _ in 0..16 {
            match dec.get_picture() {
                Ok(pic) => {
                    if let Some(rgb) = picture_to_rgb(&pic) {
                        return Some(image::DynamicImage::ImageRgb8(rgb));
                    }
                }
                Err(_) => break,
            }
        }
        // Добираем отложенное (TryAgain внутри send_data) и delayed-кадры.
        let _ = dec.send_pending_data();
        for _ in 0..16 {
            match dec.get_picture() {
                Ok(pic) => {
                    if let Some(rgb) = picture_to_rgb(&pic) {
                        return Some(image::DynamicImage::ImageRgb8(rgb));
                    }
                }
                Err(_) => break,
            }
        }
        dec.flush();
        for _ in 0..16 {
            match dec.get_picture() {
                Ok(pic) => {
                    if let Some(rgb) = picture_to_rgb(&pic) {
                        return Some(image::DynamicImage::ImageRgb8(rgb));
                    }
                }
                Err(_) => break,
            }
        }
        None
    }));
    match res {
        Ok(img) => img,
        Err(_) => None,
    }
}

/// Извлечь первый кадр AV1-видео (mp4/mov/webm/mkv) целиком.
/// Возвращает `None`, если это не AV1 или поток битый.
pub fn extract_frame(path: &Path) -> Option<image::DynamicImage> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    let n = meta.len().min(SCAN_LIMIT) as usize;
    let data = read_prefix(path, n)?;
    let ext_is_webm = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_lowercase().as_str(), "webm" | "mkv"))
        .unwrap_or(false);
    if ext_is_webm || looks_like_ebml(&data) {
        let (cp, frames) = webm_frames(&data);
        if cp.is_empty() && frames.is_empty() {
            return None;
        }
        let obu = obu_stream_from_webm(&cp, &frames)?;
        return decode_obu_first_frame(&obu);
    }
    // MP4/MOV: нужен av01/av1C, иначе это не наш кодек — быстрый выход.
    let cfg = find_av1c(&data)?;
    let mdat = find_mdat_payloads(&data);
    if mdat.is_empty() && cfg.config_obus.is_empty() {
        return None;
    }
    let obu = obu_stream_from_mdat(&mdat, &cfg.config_obus)?;
    decode_obu_first_frame(&obu)
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
    // Масштабируем к портретному превью сторис, как остальные ветки.
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

    fn sample_av1c(config_obus: &[u8]) -> Vec<u8> {
        // marker|version=0x81, profile0/level0, 8бит 4:2:0, reserved+no delay.
        let mut v = vec![0x81, 0x00, 0x0C, 0x00];
        v.extend_from_slice(config_obus);
        v
    }

    /// Минимальный валидный OBU sequence header: header 0x0A + leb128 len + payload.
    fn seq_obu(payload_len: usize) -> Vec<u8> {
        let mut v = vec![0x0Au8];
        // leb128 размера.
        let mut n = payload_len;
        loop {
            let mut b = (n & 0x7F) as u8;
            n >>= 7;
            if n != 0 {
                b |= 0x80;
            }
            v.push(b);
            if n == 0 {
                break;
            }
        }
        v.extend(std::iter::repeat(0xABu8).take(payload_len));
        v
    }

    #[test]
    fn av1c_roundtrip() {
        let cfg = parse_av1c(&sample_av1c(&seq_obu(4))).expect("parse");
        assert_eq!(cfg.seq_profile, 0);
        assert!(!cfg.high_bitdepth && !cfg.twelve_bit && !cfg.monochrome);
        assert!(!cfg.config_obus.is_empty());
    }

    #[test]
    fn av1c_rejects_garbage() {
        assert!(parse_av1c(&[]).is_none());
        assert!(parse_av1c(&[0x81, 0x00]).is_none()); // коротко
        assert!(parse_av1c(&[0x01, 0x00, 0x0C, 0x00, 0x0A]).is_none()); // marker != 1
        assert!(parse_av1c(&[0x81, 0xFF, 0x0C, 0x00]).is_none()); // profile 7
    }

    fn sample_mp4(av1c: &[u8], mdat: &[u8]) -> Vec<u8> {
        // moov/trak/mdia/minf/stbl/stsd/av01/av1C
        let mut av01_inner = vec![0u8; 78];
        av01_inner.extend_from_slice(&boxed(b"av1C", av1c));
        let av01 = boxed(b"av01", &av01_inner);
        let mut stsd_payload = vec![0u8; 8];
        stsd_payload.extend_from_slice(&av01);
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

    #[test]
    fn find_av1c_in_synthetic_mp4() {
        let av1c = sample_av1c(&seq_obu(4));
        let mp4 = sample_mp4(&av1c, &seq_obu(8));
        let cfg = find_av1c(&mp4).expect("av1C found");
        assert_eq!(cfg.seq_profile, 0);
        assert!(!cfg.config_obus.is_empty());
        // H.264-файл av1C не содержит.
        assert!(find_av1c(b"avc1....not av1................").is_none());
    }

    #[test]
    fn obu_walk_counts_units() {
        let mut stream = seq_obu(4);
        stream.extend_from_slice(&[0x10]); // temporal delimiter без размера
        stream.extend_from_slice(&seq_obu(2));
        let (n, consumed) = parse_obu_stream(&stream);
        assert_eq!(n, 3);
        assert_eq!(consumed, stream.len());
        // Мусор — ноль юнитов.
        assert_eq!(parse_obu_stream(&[0xFF, 0x00, 0x11]).0, 0);
        assert_eq!(parse_obu_stream(&[]).0, 0);
    }

    #[test]
    fn leb128_roundtrip() {
        assert_eq!(read_leb128(&[0x00]), Some((0, 1)));
        assert_eq!(read_leb128(&[0xAC, 0x02]), Some((300, 2)));
        assert_eq!(read_leb128(&[0x80]), None); // усечено
        assert_eq!(read_leb128(&[]), None);
    }

    #[test]
    fn length_prefixed_samples_stripped() {
        let obu = seq_obu(6);
        let mut sample = Vec::new();
        sample.extend_from_slice(&(obu.len() as u32).to_be_bytes());
        sample.extend_from_slice(&obu);
        sample.extend_from_slice(&(obu.len() as u32).to_be_bytes());
        sample.extend_from_slice(&obu);
        let norm = normalize_mp4_sample(&sample);
        assert_eq!(norm.len(), obu.len() * 2);
        let (n, _) = parse_obu_stream(&norm);
        assert_eq!(n, 2);
        // Сырой поток не трогаем.
        assert_eq!(normalize_mp4_sample(&obu), obu);
    }

    #[test]
    fn obu_stream_needs_something() {
        assert!(obu_stream_from_mdat(&[], &[]).is_none());
        let obu = seq_obu(6);
        let mdat = vec![obu.as_slice()];
        let stream = obu_stream_from_mdat(&mdat, &[]).expect("stream");
        assert_eq!(parse_obu_stream(&stream).0, 1);
    }

    #[test]
    fn decode_rejects_garbage_without_panic() {
        assert!(decode_obu_first_frame(&[]).is_none());
        assert!(decode_obu_first_frame(&[0u8; 4]).is_none());
        assert!(decode_obu_first_frame(&vec![0xFF; 4096]).is_none());
        // Валидные OBU-заголовки, но мусорный payload — декодер сдаётся без паники.
        let mut fake = seq_obu(64);
        fake.extend_from_slice(&seq_obu(64));
        assert!(decode_obu_first_frame(&fake).is_none());
    }

    #[test]
    fn extract_frame_missing_or_foreign_is_none() {
        assert!(extract_frame(Path::new("/nope/no-video.mp4")).is_none());
        // Синтетический H.264-mp4 (без av01) — не наш кодек.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("fake-h264.mp4");
        let mut avc1_inner = vec![0u8; 78];
        avc1_inner.extend_from_slice(&boxed(b"avcC", &[0x01, 0x42, 0x00]));
        let file = boxed(b"ftyp", b"isom");
        std::fs::write(&f, &file).unwrap();
        // ftyp-only: нет moov → None без паники.
        assert!(extract_frame(&f).is_none());
    }

    #[test]
    fn yuv_pixel_black_white_red() {
        // Limited: чёрный (16,128,128), белый (235,128,128).
        let b = yuv_to_rgb_pixel(16, 128, 128, false);
        assert!(b.iter().all(|&c| c <= 2), "{b:?}");
        let w = yuv_to_rgb_pixel(235, 128, 128, false);
        assert!(w.iter().all(|&c| c >= 250), "{w:?}");
        // Full: чёрный (0,128,128), белый (255,128,128).
        let bf = yuv_to_rgb_pixel(0, 128, 128, true);
        assert!(bf.iter().all(|&c| c <= 2), "{bf:?}");
        let wf = yuv_to_rgb_pixel(255, 128, 128, true);
        assert!(wf.iter().all(|&c| c >= 250), "{wf:?}");
        // Красный limited ≈ (81,90,240).
        let r = yuv_to_rgb_pixel(81, 90, 240, false);
        assert!(r[0] > 200 && r[1] < 60 && r[2] < 60, "{r:?}");
        // Красный full ≈ (76,85,255) → R высок, G/B низки.
        let rf = yuv_to_rgb_pixel(76, 85, 255, true);
        assert!(rf[0] > 200 && rf[1] < 80 && rf[2] < 80, "{rf:?}");
    }

    // --- WebM/EBML ---

    fn ebml_elem(id: &[u8], payload: &[u8]) -> Vec<u8> {
        fn vint(mut v: usize) -> Vec<u8> {
            // Кодируем минимальным VINT.
            for len in 1..=8 {
                let max = (1usize << (7 * len)) - 2; // -1 резерв под unknown
                if v <= max {
                    let mut out = vec![0u8; len];
                    for b in out.iter_mut().rev() {
                        *b = (v & 0xFF) as u8;
                        v >>= 8;
                    }
                    out[0] |= 0x80 >> (len - 1);
                    // Проверка: маркер не затирает данные.
                    return out;
                }
            }
            unreachable!()
        }
        let mut v = Vec::new();
        v.extend_from_slice(id);
        v.extend_from_slice(&vint(payload.len()));
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn ebml_vint_roundtrip() {
        assert_eq!(ebml_read_vint(&[0x81], 0), Some((1, 1)));
        assert_eq!(ebml_read_vint(&[0x40, 0x2C], 0), Some((0x2C, 2)));
        assert_eq!(ebml_read_vint(&[0xFF], 0), Some((u64::MAX, 1))); // unknown
        assert_eq!(ebml_read_vint(&[], 0), None);
        assert_eq!(
            ebml_read_id(&[0x1A, 0x45, 0xDF, 0xA3], 0),
            Some((0x1A45DFA3, 4))
        );
    }

    #[test]
    fn webm_frames_roundtrip() {
        // CodecPrivate с фейковым sequence header OBU.
        let cp = seq_obu(4);
        let tracks = ebml_elem(&[0x16, 0x54, 0xAE, 0x6B], &ebml_elem(&[0x63, 0xA2], &cp));
        // SimpleBlock: track=1 (0x81), timecode 0, flags keyframe 0x80, payload OBU.
        let mut sb = vec![0x81, 0x00, 0x00, 0x80];
        let obu = seq_obu(8);
        sb.extend_from_slice(&obu);
        let cluster = ebml_elem(&[0x1F, 0x43, 0xB6, 0x75], &ebml_elem(&[0xA3], &sb));
        let mut file = ebml_elem(&[0x1A, 0x45, 0xDF, 0xA3], b"head");
        file.extend_from_slice(&ebml_elem(
            &[0x18, 0x53, 0x80, 0x67],
            &[tracks, cluster].concat(),
        ));
        assert!(looks_like_ebml(&file));
        let (got_cp, frames) = webm_frames(&file);
        assert_eq!(got_cp, cp);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], obu);
        let stream = obu_stream_from_webm(&got_cp, &frames).unwrap();
        assert_eq!(parse_obu_stream(&stream).0, 2);
    }

    #[test]
    fn webm_skips_laced_blocks() {
        // flags 0x86 = лейсинг Xiph → пропускаем.
        let mut sb = vec![0x81, 0x00, 0x00, 0x86, 0x01];
        sb.extend_from_slice(&seq_obu(4));
        let cluster = ebml_elem(&[0x1F, 0x43, 0xB6, 0x75], &ebml_elem(&[0xA3], &sb));
        let file = ebml_elem(&[0x18, 0x53, 0x80, 0x67], &cluster);
        let (_, frames) = webm_frames(&file);
        assert!(frames.is_empty());
        assert!(obu_stream_from_webm(&[], &[]).is_none());
    }

    #[test]
    fn poster_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(poster_from_video(
            dir.path().join("no.mp4").as_path(),
            dir.path().join("no.poster.jpg").as_path()
        )
        .is_none());
    }
}
