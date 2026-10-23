//! ONNX-скоринг сторис за фичей `ort` (Microsoft ONNX Runtime).
//!
//! Как подключить свою модель:
//! 1. Соберите проект с фичей: `cargo run --features ort`.
//! 2. В `Настройки → 🧠 ONNX` укажите путь к `ad_model.onnx`
//!    (бинарный классификатор «реклама/не реклама») и включите тумблер.
//! 3. Имя входа: пусто = автоопределение (первый float/u8/int8/f16 тензор
//!    ранга 4, иначе ранга 3, иначе первый тензор). Если автоопределение
//!    промахнулось — впишите имя входа вручную (видно в логе при загрузке).
//! 4. Execution provider: `auto` (по умолчанию, CPU), `cpu`, `coreml`,
//!    `cuda`, `tensorrt`, `openvino`, `onednn`, `nnapi`, `directml`, `xnnpack`.
//!    В UI видны только EP, собранные в бинарнике; для GPU/ускорителей
//!    соберите с доп. фичей (`--features ort,ort-coreml` на macOS,
//!    `ort,ort-cuda` на NVIDIA и т.д.). Недоступный в рантайме EP молча
//!    откатывается на CPU (`fail_silently` в ORT) с варнингом в логе —
//!    скор не выдумывается, модель просто едет на CPU.
//! 5. Без модели или без фичи автоматически работает CLIP-лайт
//!    (`ai/clip.rs`, чистый Rust) — деградация graceful, без ошибок.
//!
//! Streaming/batch: `try_onnx_boost_many` считает сразу пачку кадров,
//! держа сессию открытой один раз (а не open/run/close на кадр), а
//! persistent-кэш в SQLite (`onnx_cache`: sha картинки + ключ модели)
//! вообще пропускает повторный инференс для неизменных кадров —
//! обновление ленты сторис после первого прогона почти бесплатное.
//! Ключ модели включает mtime файла + провайдер + имя входа:
//! заменили модель — кэш инвалидируется сам.
//!
//! Ожидаемая модель: вход — RGB-изображение (NCHW или NHWC, float32 0..1,
//! uint8 0..255, int8 −128..127, int32/int64, float16 0..1; динамические
//! dims считаются 224), выход — первый тензор: скаляр (логит/вероятность),
//! 2 класса (softmax → p класса 1) или вектор (max → sigmoid при
//! необходимости). Квантованные выходы u8/i8 маппятся в 0..1 линейно
//! (best-effort, помечается в причине). Буст: `(p-0.5)*0.6`, clamp 0..0.30.

use std::path::Path;

// Принудительно линкуем ort, когда фича включена (проверяется на CI).
#[cfg(feature = "ort")]
use ort as _;

/// Собран ли бинарник с ONNX Runtime.
pub const fn ort_compiled() -> bool {
    cfg!(feature = "ort")
}

/// Доступные execution providers для UI (порядок = порядок в ComboBox).
pub const PROVIDER_CHOICES: &[&str] = &[
    "auto", "cpu", "coreml", "cuda", "tensorrt", "openvino", "onednn", "nnapi", "directml",
    "xnnpack",
];

/// Канонизация выбора EP из конфига/UI. Неизвестное → "auto".
pub fn normalize_provider(s: &str) -> &'static str {
    let t = s.trim().to_lowercase();
    if t.is_empty() || t == "default" {
        return "auto";
    }
    for c in PROVIDER_CHOICES {
        if *c == t {
            return c;
        }
    }
    "auto"
}

/// Собран ли данный EP в текущем бинарнике (CPU/auto — всегда).
/// Несобранный EP из UI выбрать нельзя — список строит `available_providers`.
pub fn ep_compiled_in(name: &str) -> bool {
    match normalize_provider(name) {
        "auto" | "cpu" => true,
        #[cfg(feature = "ort-coreml")]
        "coreml" => true,
        #[cfg(feature = "ort-cuda")]
        "cuda" => true,
        #[cfg(feature = "ort-tensorrt")]
        "tensorrt" => true,
        #[cfg(feature = "ort-openvino")]
        "openvino" => true,
        #[cfg(feature = "ort-onednn")]
        "onednn" => true,
        #[cfg(feature = "ort-nnapi")]
        "nnapi" => true,
        #[cfg(feature = "ort-directml")]
        "directml" => true,
        #[cfg(feature = "ort-xnnpack")]
        "xnnpack" => true,
        _ => false,
    }
}

/// Провайдеры, доступные в этом бинарнике (для ComboBox в настройках).
pub fn available_providers() -> Vec<&'static str> {
    PROVIDER_CHOICES
        .iter()
        .copied()
        .filter(|c| ep_compiled_in(c))
        .collect()
}

/// Человекочитаемый статус для UI.
pub fn ort_status(model_path: &str, enabled: bool, provider: &str) -> String {
    if !ort_compiled() {
        return "собран без фичи `ort` — работает CLIP-лайт".into();
    }
    if !enabled {
        return "ort собран, но выключен в настройках — работает CLIP-лайт".into();
    }
    if model_path.trim().is_empty() {
        return "ort собран, но путь к модели пуст — укажите ad_model.onnx".into();
    }
    if !Path::new(model_path).exists() {
        return format!("модель не найдена: {model_path}");
    }
    let ep = normalize_provider(provider);
    if ep == "auto" {
        "модель найдена — ONNX активен (EP: auto→CPU)".into()
    } else {
        format!("модель найдена — ONNX активен (EP: {ep}, fallback на CPU)")
    }
}

/// Ключ модели для streaming-кэша: путь + mtime + провайдер + имя входа.
/// Замена файла модели (новый mtime) инвалидирует кэш автоматически.
pub fn model_key_of(model_path: &str, provider: &str, input_override: &str) -> String {
    let mtime = std::fs::metadata(model_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{}|{mtime}|{}|{}",
        normalize_provider(provider),
        input_override.trim(),
        model_path
    )
}

/// Стабильный sha картинки (геометрия + сырые RGBA-пиксели) для кэша.
pub fn image_sha(image: &image::DynamicImage) -> String {
    use sha2::{Digest, Sha256};
    let rgba = image.to_rgba8();
    let mut h = Sha256::new();
    h.update(rgba.width().to_le_bytes());
    h.update(rgba.height().to_le_bytes());
    h.update(rgba.as_raw());
    hex::encode(h.finalize())
}

/// Пробует посчитать ONNX-буст 0..0.30. `None` = fallback на CLIP-лайт.
/// Сначала смотрит persistent-кэш SQLite (`db`), инференс — только при промахе.
pub fn try_onnx_boost(
    db: Option<&crate::db::Db>,
    model_path: &str,
    enabled: bool,
    image: &image::DynamicImage,
    input_override: &str,
    provider: &str,
) -> Option<(f32, Vec<String>)> {
    try_onnx_boost_many(db, model_path, enabled, &[Some(image)], input_override, provider)
        .into_iter()
        .next()
        .flatten()
}

/// Пачка входных изображений для streaming-инференса.
pub type ImageBatch<'a> = &'a [Option<&'a image::DynamicImage>];
/// Результат ONNX: буст + причины (None — модель не применима).
pub type OnnxBoost = Option<(f32, Vec<String>)>;

/// Batch-версия: пачка кадров за одно удержание сессии + кэш на каждый.
/// Возвращает по элементу на вход (порядок сохранён).
pub fn try_onnx_boost_many(
    db: Option<&crate::db::Db>,
    model_path: &str,
    enabled: bool,
    images: ImageBatch<'_>,
    input_override: &str,
    provider: &str,
) -> Vec<OnnxBoost> {
    let n = images.len();
    if !enabled || model_path.trim().is_empty() || !Path::new(model_path).exists() {
        return vec![None; n];
    }
    let key = model_key_of(model_path, provider, input_override);
    let mut out: Vec<Option<(f32, Vec<String>)>> = vec![None; n];
    // Фаза 1: хиты кэша (модель не дёргаем вообще).
    let mut misses: Vec<(usize, &image::DynamicImage, String)> = Vec::new();
    for (i, img_opt) in images.iter().enumerate() {
        let Some(img) = img_opt else { continue };
        let sha = image_sha(img);
        if let Some((p, boost, reasons)) = db.and_then(|d| d.onnx_cache_get(&sha, &key)) {
            if boost > 0.001 {
                let rs: Vec<String> = reasons
                    .split("; ")
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                out[i] = Some((boost, rs));
            } else {
                out[i] = Some((0.0, vec![]));
            }
            let _ = p;
        } else {
            misses.push((i, img, sha));
        }
    }
    if misses.is_empty() {
        return out;
    }
    // Фаза 2: промахи — один проход сессии по всем кадрам (streaming).
    // Ок(Some) кэшируем (включая уверенный негатив), Err — нет (транзиент).
    let miss_imgs: Vec<&image::DynamicImage> = misses.iter().map(|(_, img, _)| *img).collect();
    let results = run_many(model_path, &miss_imgs, input_override, provider);
    for ((i, _, sha), res) in misses.into_iter().zip(results) {
        match res {
            Ok(Some((boost, reasons))) => {
                let p = reasons
                    .iter()
                    .find_map(|r| r.rsplit("p=").next()?.trim().parse::<f32>().ok())
                    .unwrap_or(0.5);
                if let Some(d) = db {
                    d.onnx_cache_put(&sha, &key, p, boost, &reasons.join("; "));
                }
                out[i] = Some((boost, reasons));
            }
            Ok(None) => {
                if let Some(d) = db {
                    d.onnx_cache_put(&sha, &key, 0.0, 0.0, "");
                }
                out[i] = Some((0.0, vec![]));
            }
            Err(e) => {
                tracing::debug!("ONNX streaming miss failed (не кэшируем): {e}");
            }
        }
    }
    out
}

/// Один проход пачки кадров. `Ok(..)` — можно кэшировать, `Err(..)` — нет.
#[cfg(feature = "ort")]
fn run_many(
    model_path: &str,
    images: &[&image::DynamicImage],
    input_override: &str,
    provider: &str,
) -> Vec<Result<Option<(f32, Vec<String>)>, String>> {
    backend::infer_many(model_path, images, input_override, provider)
}

#[cfg(not(feature = "ort"))]
fn run_many(
    model_path: &str,
    images: &[&image::DynamicImage],
    input_override: &str,
    provider: &str,
) -> Vec<Result<Option<(f32, Vec<String>)>, String>> {
    let _ = (model_path, input_override, provider);
    vec![Err("ort не собран".to_string()); images.len()]
}

// ---------------------------------------------------------------------------
// Чистые помощники (без ort): раскладка входа, калибровка выхода, FP16.
// Покрыты тестами.
// ---------------------------------------------------------------------------

/// Раскладка входа по shape: (h, w, nchw). Динамические (<=0) dims → 224.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLayout {
    Nchw,
    Nhwc,
}

pub fn derive_layout(shape: &[i64]) -> (usize, usize, InputLayout) {
    const D: usize = 224;
    let dim = |i: usize| -> usize {
        shape
            .get(i)
            .copied()
            .filter(|&d| d > 0)
            .map(|d| d as usize)
            .unwrap_or(D)
    };
    match shape.len() {
        4 => {
            // [n,c,h,w] или [n,h,w,c]
            if matches!(shape.get(1), Some(1) | Some(3)) {
                (dim(2), dim(3), InputLayout::Nchw)
            } else if matches!(shape.get(3), Some(1) | Some(3)) {
                (dim(1), dim(2), InputLayout::Nhwc)
            } else {
                (D, D, InputLayout::Nchw)
            }
        }
        3 => {
            // [c,h,w] или [h,w,c]
            if matches!(shape.first(), Some(1) | Some(3)) {
                (dim(1), dim(2), InputLayout::Nchw)
            } else if matches!(shape.last(), Some(1) | Some(3)) {
                (dim(0), dim(1), InputLayout::Nhwc)
            } else {
                (D, D, InputLayout::Nchw)
            }
        }
        _ => (D, D, InputLayout::Nchw),
    }
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Вероятность рекламы 0..1 → буст 0..0.30.
pub fn prob_to_boost(p: f32) -> f32 {
    ((p - 0.5) * 0.6).clamp(0.0, 0.30)
}

/// Сырой выход модели → p(реклама): скаляр/2 класса/вектор.
pub fn output_to_prob(values: &[f32]) -> f32 {
    match values {
        [] => 0.0,
        [v] => {
            if (0.0..=1.0).contains(v) {
                *v
            } else {
                sigmoid(*v)
            }
        }
        [a, b] => {
            let (m, e0, e1) = (a.max(*b), (a - b.max(*b)).exp(), (b - a.max(*b)).exp());
            let _ = m;
            e1 / (e0 + e1)
        }
        v => {
            let m = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            if (0.0..=1.0).contains(&m) {
                m
            } else {
                sigmoid(m)
            }
        }
    }
}

/// Квантованный ли dtype входа/выхода (имя как в `TensorElementType::Display`).
pub fn is_quantized_dtype_name(name: &str) -> bool {
    matches!(
        name,
        "i8" | "u8"
            | "i4"
            | "u4"
            | "f8_e4m3fn"
            | "f8_e4m3fnuz"
            | "f8_e5m2"
            | "f8_e5m2fnuz"
            | "i16"
            | "u16"
    )
}

/// Пониженная точность (не квантование, но тоже «лёгкая» модель).
pub fn is_low_precision_dtype_name(name: &str) -> bool {
    matches!(name, "f16" | "bf16")
}

/// IEEE-754 binary16 биты → f32. Чистый Rust, без `half` (работает и без фичи `ort`).
pub fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1F) as i32;
    let mant = (bits & 0x3FF) as u32;
    let f32bits = if exp == 0 {
        if mant == 0 {
            sign << 31
        } else {
            // Субнормаль: нормализуем.
            let mut e = -14i32;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x3FF;
            ((sign << 31) as i32 + ((e + 127) << 23)) as u32 | (m << 13)
        }
    } else if exp == 31 {
        (sign << 31) | (0xFF << 23) | (mant << 13)
    } else {
        (sign << 31) | (((exp + 112) as u32) << 23) | (mant << 13)
    };
    f32::from_bits(f32bits)
}

/// f32 → IEEE-754 binary16 биты (round-to-nearest-even, inf/nan сохраняются).
pub fn f32_to_f16_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xFF) as i32;
    let mant = b & 0x7F_FFFF;
    if exp == 255 {
        // inf/nan → inf (nan с полезной нагрузкой для отличия от inf).
        return sign | 0x7C00 | if mant != 0 { 0x0200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7C00; // переполнение → inf
    }
    if e <= 0 {
        // Субнормаль или ноль.
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000; // 24 значащих бита
        let shift = (14 - e) as u32;
        let mut half_mant = (m >> shift) as u16;
        let rest = m & ((1u32 << shift) - 1);
        let half_bit = 1u32 << (shift - 1);
        if rest > half_bit || (rest == half_bit && (half_mant & 1) == 1) {
            half_mant += 1;
        }
        return sign | half_mant;
    }
    let mut half_mant = (mant >> 13) as u16;
    let rest = mant & 0x1FFF;
    if rest > 0x1000 || (rest == 0x1000 && (half_mant & 1) == 1) {
        half_mant += 1;
        if half_mant == 0x400 {
            // Перенос в экспоненту.
            return sign | (((e + 1) as u16) << 10);
        }
    }
    sign | ((e as u16) << 10) | half_mant
}

/// Квантованный выход u8 → 0..1 (best-effort: линейный маппинг без scale/zero-point).
#[cfg_attr(not(feature = "ort"), allow(dead_code))]
pub fn dequantize_u8_row(vals: &[u8]) -> Vec<f32> {
    vals.iter().map(|v| *v as f32 / 255.0).collect()
}

/// Квантованный выход i8 → 0..1 (best-effort).
#[cfg_attr(not(feature = "ort"), allow(dead_code))]
pub fn dequantize_i8_row(vals: &[i8]) -> Vec<f32> {
    vals.iter().map(|v| (*v as f32 + 128.0) / 255.0).collect()
}

// ---------------------------------------------------------------------------
// ort-бэкенд (только с фичей).
// ---------------------------------------------------------------------------

#[cfg(feature = "ort")]
mod backend {
    use super::*;

    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    static CACHE: std::sync::Mutex<Option<(String, ort::session::Session)>> =
        std::sync::Mutex::new(None);

    fn env() -> Result<&'static ort::environment::Environment, String> {
        if let Some(e) = ENV.get() {
            return Ok(e);
        }
        match ort::init().build() {
            Ok(e) => Ok(ENV.get_or_init(|| e)),
            Err(first) => {
                // Гонка: второй build падает, но первый уже сохранил env.
                if let Some(e) = ENV.get() {
                    Ok(e)
                } else {
                    Err(format!("ort env: {first}"))
                }
            }
        }
    }

    fn with_session<T>(
        model_path: &str,
        provider: &str,
        f: impl FnOnce(&mut ort::session::Session) -> Result<T, String>,
    ) -> Result<T, String> {
        let e = env()?;
        let mut guard = CACHE.lock().map_err(|_| "ort cache poisoned".to_string())?;
        let key = format!("{model_path}\u{1f}{}", super::normalize_provider(provider));
        let stale = guard.as_ref().map(|(p, _)| p != &key).unwrap_or(true);
        if stale {
            let session = build_session(e, model_path, provider)?;
            log_model_io(&session);
            *guard = Some((key, session));
        }
        let (_, s) = guard.as_mut().expect("just cached");
        f(s)
    }

    fn build_session(
        e: &ort::environment::Environment,
        model_path: &str,
        provider: &str,
    ) -> Result<ort::session::Session, String> {
        let builder =
            ort::session::Session::builder(e).map_err(|err| format!("builder: {err}"))?;
        let ep = super::normalize_provider(provider);
        if !super::ep_compiled_in(ep) {
            tracing::warn!(
                "ONNX EP {ep} не собран в этом бинарнике (нужна фича ort-{ep}), используется CPU"
            );
        }
        // Каждый EP-тип существует, только если включена его фича ort-*.
        // Несобранные имена сюда не доходят (UI показывает лишь available_providers),
        // но match всё равно total через fallback-ветку.
        let with_eps = match ep {
            #[cfg(feature = "ort-coreml")]
            "coreml" => builder.with_execution_providers([ort::ep::CoreML::default().build()]),
            #[cfg(feature = "ort-cuda")]
            "cuda" => builder.with_execution_providers([ort::ep::CUDA::default().build()]),
            #[cfg(feature = "ort-tensorrt")]
            "tensorrt" => builder.with_execution_providers([ort::ep::TensorRT::default().build()]),
            #[cfg(feature = "ort-openvino")]
            "openvino" => builder.with_execution_providers([ort::ep::OpenVINO::default().build()]),
            #[cfg(feature = "ort-onednn")]
            "onednn" => builder.with_execution_providers([ort::ep::OneDNN::default().build()]),
            #[cfg(feature = "ort-nnapi")]
            "nnapi" => builder.with_execution_providers([ort::ep::NNAPI::default().build()]),
            #[cfg(feature = "ort-directml")]
            "directml" => builder.with_execution_providers([ort::ep::DirectML::default().build()]),
            #[cfg(feature = "ort-xnnpack")]
            "xnnpack" => builder.with_execution_providers([ort::ep::XNNPACK::default().build()]),
            _ => builder.with_execution_providers([ort::ep::CPU::default().build()]),
        };
        // Недоступный в рантайме EP: warn + CPU (fail_silently внутри ORT уже
        // откатил регистрацию, ошибку билдера разворачиваем в рабочий builder).
        let mut builder = match with_eps {
            Ok(b) => {
                if ep != "auto" {
                    tracing::info!("ONNX EP запрошен: {ep}");
                }
                b
            }
            Err(err) => {
                tracing::warn!("ONNX EP {ep} недоступен в рантайме, fallback на CPU: {err}");
                err.recover()
            }
        };
        builder
            .commit_from_file(model_path)
            .map_err(|err| format!("load {model_path}: {err}"))
    }

    fn log_model_io(session: &ort::session::Session) {
        let ins: Vec<String> = session
            .inputs()
            .iter()
            .map(|o| {
                let q = match o.dtype() {
                    ort::value::ValueType::Tensor { ty, .. } => {
                        let n = ty.to_string();
                        if super::is_quantized_dtype_name(&n) {
                            " [квантованная]"
                        } else if super::is_low_precision_dtype_name(&n) {
                            " [FP16]"
                        } else {
                            ""
                        }
                    }
                    _ => "",
                };
                format!("{}: {:?}{q}", o.name(), o.dtype())
            })
            .collect();
        let outs: Vec<String> = session
            .outputs()
            .iter()
            .map(|o| format!("{}: {:?}", o.name(), o.dtype()))
            .collect();
        tracing::info!("ONNX модель загружена. Входы: [{}]. Выходы: [{}]", ins.join(", "), outs.join(", "));
    }

    /// Пачка кадров за одно удержание сессии: открываем сессию один раз,
    /// затем `run_once` на кадр. Ошибка сессии — Err на всю пачку;
    /// ошибка кадра — Err на этот кадр (не кэшируется вызывающим).
    pub(super) fn infer_many(
        model_path: &str,
        images: &[&image::DynamicImage],
        input_override: &str,
        provider: &str,
    ) -> Vec<Result<Option<(f32, Vec<String>)>, String>> {
        match with_session(model_path, provider, |s| {
            let mut out = Vec::with_capacity(images.len());
            for img in images {
                out.push(run_once(s, img, model_path, input_override));
            }
            Ok(out)
        }) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("ONNX fallback на CLIP-лайт: {e}");
                vec![Err(e); images.len()]
            }
        }
    }

    fn run_once(
        session: &mut ort::session::Session,
        image: &image::DynamicImage,
        model_path: &str,
        input_override: &str,
    ) -> Result<Option<(f32, Vec<String>)>, String> {
        use ort::value::{Tensor, TensorElementType, ValueType};

        // 1. Выбор входа.
        let inputs = session.inputs();
        if inputs.is_empty() {
            return Err("у модели нет входов".into());
        }
        let pick = |o: &ort::value::Outlet| -> bool {
            matches!(o.dtype(), ValueType::Tensor { .. })
        };
        let idx = if !input_override.trim().is_empty() {
            inputs
                .iter()
                .position(|o| o.name() == input_override.trim() && pick(o))
                .ok_or_else(|| format!("вход `{}` не найден среди тензоров", input_override.trim()))?
        } else {
            inputs
                .iter()
                .position(|o| {
                    pick(o) && matches!(o.dtype(), ValueType::Tensor { shape, .. } if shape.len() == 4)
                })
                .or_else(|| {
                    inputs.iter().position(|o| {
                        pick(o) && matches!(o.dtype(), ValueType::Tensor { shape, .. } if shape.len() == 3)
                    })
                })
                .or_else(|| inputs.iter().position(pick))
                .ok_or_else(|| "нет тензорных входов".to_string())?
        };
        let input = &inputs[idx];
        let input_name = input.name().to_string();
        let (shape, ty) = match input.dtype() {
            ValueType::Tensor { ty, shape, .. } => (shape.clone(), *ty),
            _ => return Err("вход не тензор".into()),
        };
        let shape_dims: Vec<i64> = shape.iter().copied().collect();
        let (h, w, layout) = super::derive_layout(&shape_dims);
        let c = match layout {
            super::InputLayout::Nchw => shape_dims.get(1).copied().filter(|&d| d == 1 || d == 3).unwrap_or(3),
            super::InputLayout::Nhwc => shape_dims.last().copied().filter(|&d| d == 1 || d == 3).unwrap_or(3),
        } as usize;

        // 2. Препроцессинг (нормализованные 0..1), затем каст под dtype входа.
        let rgb = image.to_rgb8();
        let small =
            image::imageops::resize(&rgb, w as u32, h as u32, image::imageops::FilterType::Triangle);
        let px = small.as_raw();
        let mut data_f32 = Vec::with_capacity(c * h * w);
        match layout {
            super::InputLayout::Nchw => {
                for ch in 0..c {
                    for p in px.chunks(3) {
                        let v = p[ch.min(2)] as f32 / 255.0;
                        data_f32.push(if c == 1 {
                            // grayscale: luma
                            (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) / 255.0
                        } else {
                            v
                        });
                    }
                }
            }
            super::InputLayout::Nhwc => {
                if c == 1 {
                    for p in px.chunks(3) {
                        data_f32.push(
                            (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) / 255.0,
                        );
                    }
                } else {
                    for p in px.chunks(3) {
                        data_f32.push(p[0] as f32 / 255.0);
                        data_f32.push(p[1] as f32 / 255.0);
                        data_f32.push(p[2] as f32 / 255.0);
                    }
                }
            }
        }
        let dtype_name = ty.to_string();
        let quantized_note = if super::is_quantized_dtype_name(&dtype_name) {
            "квант. "
        } else if super::is_low_precision_dtype_name(&dtype_name) {
            "FP16 "
        } else {
            ""
        };

        // 3. Инференс: float32 / uint8 (квантованные) / int8 / int32 / int64 / float16.
        let shape_vec: Vec<usize> = match layout {
            super::InputLayout::Nchw => vec![1usize, c, h, w],
            super::InputLayout::Nhwc => vec![1usize, h, w, c],
        };
        let outputs = match ty {
            TensorElementType::Float32 => {
                let t = Tensor::<f32>::from_array((shape_vec, data_f32))
                    .map_err(|e| format!("input tensor: {e}"))?;
                session
                    .run(vec![(input_name.clone(), t.upcast())])
                    .map_err(|e| format!("run: {e}"))?
            }
            TensorElementType::Uint8 => {
                let data_u8: Vec<u8> = data_f32.into_iter().map(|v| (v * 255.0) as u8).collect();
                let t = Tensor::<u8>::from_array((shape_vec, data_u8))
                    .map_err(|e| format!("input tensor u8: {e}"))?;
                session
                    .run(vec![(input_name.clone(), t.upcast())])
                    .map_err(|e| format!("run: {e}"))?
            }
            TensorElementType::Int8 => {
                let data_i8: Vec<i8> = data_f32
                    .into_iter()
                    .map(|v| ((v * 255.0 - 128.0).round().clamp(-128.0, 127.0)) as i8)
                    .collect();
                let t = Tensor::<i8>::from_array((shape_vec, data_i8))
                    .map_err(|e| format!("input tensor i8: {e}"))?;
                session
                    .run(vec![(input_name.clone(), t.upcast())])
                    .map_err(|e| format!("run: {e}"))?
            }
            TensorElementType::Int32 => {
                // Целочисленный вход без scale: сырые 0..255 (best-effort).
                let data_i32: Vec<i32> = data_f32.into_iter().map(|v| (v * 255.0) as i32).collect();
                let t = Tensor::<i32>::from_array((shape_vec, data_i32))
                    .map_err(|e| format!("input tensor i32: {e}"))?;
                session
                    .run(vec![(input_name.clone(), t.upcast())])
                    .map_err(|e| format!("run: {e}"))?
            }
            TensorElementType::Int64 => {
                let data_i64: Vec<i64> = data_f32.into_iter().map(|v| (v * 255.0) as i64).collect();
                let t = Tensor::<i64>::from_array((shape_vec, data_i64))
                    .map_err(|e| format!("input tensor i64: {e}"))?;
                session
                    .run(vec![(input_name.clone(), t.upcast())])
                    .map_err(|e| format!("run: {e}"))?
            }
            TensorElementType::Float16 => {
                let data_f16: Vec<half::f16> = data_f32
                    .into_iter()
                    .map(|v| half::f16::from_f32(v))
                    .collect();
                let t = Tensor::<half::f16>::from_array((shape_vec, data_f16))
                    .map_err(|e| format!("input tensor f16: {e}"))?;
                session
                    .run(vec![(input_name.clone(), t.upcast())])
                    .map_err(|e| format!("run: {e}"))?
            }
            other => return Err(format!("тип входа {other:?} не поддерживается (нужен f32/u8/i8/i32/i64/f16)")),
        };

        // 4. Выход → p → буст (f32, квантованные u8/i8, целые, f16).
        if outputs.len() == 0 {
            return Err("пустой выход модели".into());
        }
        let vals: Vec<f32> = if let Ok((_, v)) = outputs[0].try_extract_tensor::<f32>() {
            v.to_vec()
        } else if let Ok((_, v)) = outputs[0].try_extract_tensor::<u8>() {
            super::dequantize_u8_row(v)
        } else if let Ok((_, v)) = outputs[0].try_extract_tensor::<i8>() {
            super::dequantize_i8_row(v)
        } else if let Ok((_, v)) = outputs[0].try_extract_tensor::<i32>() {
            v.iter().map(|x| *x as f32).collect()
        } else if let Ok((_, v)) = outputs[0].try_extract_tensor::<i64>() {
            v.iter().map(|x| *x as f32).collect()
        } else if let Ok((_, v)) = outputs[0].try_extract_tensor::<half::f16>() {
            v.iter().map(|x| x.to_f32()).collect()
        } else {
            return Err("выход не f32/u8/i8/i32/i64/f16".into());
        };
        let p = super::output_to_prob(&vals);
        let boost = super::prob_to_boost(p);
        let short = Path::new(model_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| model_path.to_string());
        if boost > 0.001 {
            Ok(Some((
                boost,
                vec![format!("ONNX {quantized_note}{short} [{input_name}]: p={p:.2}")],
            )))
        } else {
            Ok(Some((0.0, vec![])))
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_returns_none() {
        let img = image::DynamicImage::new_rgb8(8, 8);
        assert!(try_onnx_boost(None, "", false, &img, "", "auto").is_none());
        assert!(try_onnx_boost(None, "/nope/model.onnx", true, &img, "", "cuda").is_none());
        // Batch: порядок и длина сохранены, всё None без модели.
        let out = try_onnx_boost_many(None, "", false, &[Some(&img), None, Some(&img)], "", "auto");
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|o| o.is_none()));
    }

    #[test]
    fn model_key_changes_with_inputs() {
        let a = model_key_of("/tmp/ad.onnx", "cpu", "");
        let b = model_key_of("/tmp/ad.onnx", "coreml", "");
        let c = model_key_of("/tmp/ad.onnx", "cpu", "input");
        let d = model_key_of("/tmp/other.onnx", "cpu", "");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a, model_key_of("/tmp/ad.onnx", "CPU", "")); // канонизация
    }

    #[test]
    fn image_sha_stable_and_distinct() {
        let a = image::DynamicImage::new_rgb8(8, 8);
        let mut px = image::RgbImage::new(8, 8);
        px.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        let b = image::DynamicImage::ImageRgb8(px);
        assert_eq!(image_sha(&a), image_sha(&a));
        assert_ne!(image_sha(&a), image_sha(&b));
    }

    #[test]
    fn batch_serves_cache_without_model() {
        // Кэш хит — даже если файла модели нет, результат из SQLite.
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Db::open_at(dir.path().join("state.db")).unwrap();
        let img = image::DynamicImage::new_rgb8(8, 8);
        let sha = image_sha(&img);
        // model_key_of на несуществующий файл: mtime=0, но ключ валиден.
        let key = model_key_of("/nope/model.onnx", "cpu", "");
        db.onnx_cache_put(&sha, &key, 0.9, 0.24, "ONNX m: p=0.9");
        // try_onnx_boost_many с несуществующей моделью отдаёт all-None,
        // поэтому кладём настоящий временный файл модели.
        let model = dir.path().join("ad.onnx");
        std::fs::write(&model, b"fake").unwrap();
        let key = model_key_of(&model.to_string_lossy(), "cpu", "");
        db.onnx_cache_put(&sha, &key, 0.9, 0.24, "ONNX m: p=0.9");
        let out = try_onnx_boost_many(
            Some(&db),
            &model.to_string_lossy(),
            true,
            &[Some(&img)],
            "",
            "cpu",
        );
        assert_eq!(out.len(), 1);
        let (boost, reasons) = out.into_iter().next().flatten().expect("cache hit");
        assert!((boost - 0.24).abs() < 1e-6);
        assert_eq!(reasons, vec!["ONNX m: p=0.9".to_string()]);
    }

    #[test]
    fn status_mentions_fallback_without_feature() {
        let s = ort_status("", false, "auto");
        assert!(!s.is_empty());
        if !ort_compiled() {
            assert!(s.contains("CLIP-лайт"));
        }
    }

    #[test]
    fn status_shows_provider() {
        // Канонизация регистра.
        assert_eq!(normalize_provider("CUDA"), "cuda");
        assert_eq!(normalize_provider("  CoreML "), "coreml");
        // Без фичи ort — ранний fallback без упоминания EP, но без паники.
        // С фичей и существующим файлом — EP виден в статусе.
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("ad.onnx");
        std::fs::write(&model, b"fake").unwrap();
        let s = ort_status(&model.to_string_lossy(), true, "CUDA");
        if ort_compiled() {
            assert!(s.contains("cuda"), "{s}");
        } else {
            assert!(s.contains("CLIP-лайт"), "{s}");
        }
    }

    #[test]
    fn providers_canonicalize() {
        assert_eq!(normalize_provider(""), "auto");
        assert_eq!(normalize_provider("default"), "auto");
        assert_eq!(normalize_provider("cpu"), "cpu");
        assert_eq!(normalize_provider("xnnpack"), "xnnpack");
        assert_eq!(normalize_provider("что-то-левое"), "auto");
        assert!(PROVIDER_CHOICES.contains(&"auto"));
        assert!(PROVIDER_CHOICES.contains(&"coreml"));
        assert!(PROVIDER_CHOICES.contains(&"cuda"));
        // CPU/auto доступны всегда; остальное — по фичам ort-*.
        // Неизвестное имя канонизируется в auto → тоже true (fallback).
        assert!(ep_compiled_in("auto"));
        assert!(ep_compiled_in("cpu"));
        assert!(ep_compiled_in("definitely-not-an-ep"));
        let avail = available_providers();
        assert!(avail.contains(&"auto") && avail.contains(&"cpu"));
        #[cfg(not(feature = "ort-coreml"))]
        assert!(!ep_compiled_in("coreml"));
        #[cfg(feature = "ort-coreml")]
        assert!(ep_compiled_in("coreml"));
    }

    #[test]
    fn layout_nchw_nhwc() {
        assert_eq!(derive_layout(&[1, 3, 224, 224]), (224, 224, InputLayout::Nchw));
        assert_eq!(derive_layout(&[1, 300, 300, 3]), (300, 300, InputLayout::Nhwc));
        assert_eq!(derive_layout(&[1, 3, -1, -1]), (224, 224, InputLayout::Nchw));
        assert_eq!(derive_layout(&[3, 128, 128]), (128, 128, InputLayout::Nchw));
        assert_eq!(derive_layout(&[64, 64, 3]), (64, 64, InputLayout::Nhwc));
        assert_eq!(derive_layout(&[]), (224, 224, InputLayout::Nchw));
    }

    #[test]
    fn prob_math() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert_eq!(prob_to_boost(0.5), 0.0);
        assert!((prob_to_boost(1.0) - 0.30).abs() < 1e-6);
        // softmax([0.2, 0.8]) = e^0.8/(e^0.2+e^0.8) ≈ 0.646
        assert!((output_to_prob(&[0.2, 0.8]) - 0.646).abs() < 0.01);
        assert_eq!(output_to_prob(&[]), 0.0);
        assert!((output_to_prob(&[2.0]) - sigmoid(2.0)).abs() < 1e-6);
    }

    #[test]
    fn quant_dtype_names() {
        assert!(is_quantized_dtype_name("i8"));
        assert!(is_quantized_dtype_name("u8"));
        assert!(is_quantized_dtype_name("f8_e4m3fn"));
        assert!(!is_quantized_dtype_name("f32"));
        assert!(is_low_precision_dtype_name("f16"));
        assert!(is_low_precision_dtype_name("bf16"));
        assert!(!is_low_precision_dtype_name("f32"));
    }

    #[test]
    fn f16_roundtrip_exact_bits() {
        assert_eq!(f32_to_f16_bits(1.0), 0x3C00);
        assert_eq!(f32_to_f16_bits(-1.0), 0xBC00);
        assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x7C00);
        assert_eq!(f32_to_f16_bits(65504.0), 0x7BFF);
        assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f32(0xBC00), -1.0);
        assert_eq!(f16_bits_to_f32(0x0000), 0.0);
        assert!(f16_bits_to_f32(0x7C00).is_infinite());
        assert!(f16_bits_to_f32(0x7E00).is_nan());
        // Субнормаль 0x0001 = 2^-24.
        assert!((f16_bits_to_f32(0x0001) - 5.9604645e-8).abs() < 1e-14);
        for v in [0.5, -0.25, 3.14159, 100.0, 1e-5, 0.00006103515625, -273.15] {
            let rt = f16_bits_to_f32(f32_to_f16_bits(v));
            let tol = v.abs() * 0.002 + 1e-7;
            assert!((rt - v).abs() <= tol, "v={v} rt={rt}");
        }
        // Ноль со знаком сохраняется.
        assert_eq!(f32_to_f16_bits(-0.0), 0x8000);
    }

    #[test]
    fn dequant_rows_linear() {
        assert_eq!(dequantize_u8_row(&[0, 255]), vec![0.0, 1.0]);
        assert_eq!(dequantize_i8_row(&[-128, 127]), vec![0.0, 1.0]);
        assert!((output_to_prob(&dequantize_u8_row(&[230])) - 230.0 / 255.0).abs() < 1e-6);
    }
}
