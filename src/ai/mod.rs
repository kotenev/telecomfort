//! ИИ-автоматизация для Сторис: обнаружение рекламы и замыливание только рекламных зон.

pub mod bktree;
pub mod blur;
pub mod clip;
pub mod cluster;
pub mod heuristic;
pub mod onnx;
pub mod phash;
pub mod visual;

use serde::{Deserialize, Serialize};

/// Оценка «рекламности» сторис 0..1
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdScore {
    /// 0.0 = точно не реклама, 1.0 = точно реклама
    pub score: f32,
    /// Человекочитаемые причины (для UI и отладки)
    pub reasons: Vec<String>,
    /// Подозрительные спаны текста (для подсветки/блюра подписей)
    pub spans: Vec<String>,
    /// Рекомендуемые зоны блюра в относительных координатах 0..1 (x,y,w,h)
    pub regions: Vec<RelRect>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl AdScore {
    pub fn clean() -> Self {
        Self {
            score: 0.0,
            reasons: vec![],
            spans: vec![],
            regions: vec![],
        }
    }

    pub fn is_ad(&self, threshold: f32) -> bool {
        self.score >= threshold
    }

    /// Добавить CLIP/ONNX-буст (0..0.30) к оценке.
    pub fn apply_clip_boost(&mut self, boost: f32, reasons: Vec<String>) {
        if boost <= 0.001 {
            return;
        }
        self.score = (self.score + boost).clamp(0.0, 0.98);
        self.reasons.extend(reasons);
    }

    /// Добавить визуальный буст (0..0.45) к текстовой оценке.
    /// Расширяет зоны блюра нижней плашкой, если визуал нашёл вёрстку.
    pub fn apply_visual_boost(&mut self, boost: f32, reasons: Vec<String>) {
        if boost <= 0.001 {
            return;
        }
        self.score = (self.score + boost).clamp(0.0, 0.98);
        self.reasons.extend(reasons);
        // Если визуал уверен, а зон ещё нет — добавляем низ кадра.
        if self.regions.is_empty() && boost >= 0.10 {
            self.regions.push(RelRect {
                x: 0.0,
                y: 0.62,
                w: 1.0,
                h: 0.38,
            });
        }
    }
}

pub trait AdDetector: Send + Sync {
    fn score_story(&self, caption: &str, has_url_entity: bool, has_qr_hint: bool) -> AdScore;
}

#[allow(unused_imports)]
pub use clip::{clip_lite_boost, embed_joint, embed_text};
pub use heuristic::HeuristicAdDetector;
#[allow(unused_imports)]
pub use onnx::{
    available_providers, ep_compiled_in, image_sha, is_low_precision_dtype_name,
    is_quantized_dtype_name, model_key_of, normalize_provider, ort_compiled, ort_status,
    try_onnx_boost, try_onnx_boost_many, PROVIDER_CHOICES,
};
pub use visual::{analyze_image, visual_ad_boost, VisualSignals};

/// Быстрый скоринг без ONNX: текст + визуал + CLIP-лайт.
/// Фаза 1 batch-пути сторис; ONNX применяется пачкой отдельно.
pub fn score_story_base(
    text_detector: &impl AdDetector,
    caption: &str,
    has_url: bool,
    image: Option<&image::DynamicImage>,
    clip_enabled: bool,
) -> AdScore {
    // 1. Визуальные сигналы (нужны и для QR-подсказки текста).
    let (sig_opt, qr) = match image {
        Some(img) => {
            let sig = analyze_image(img);
            let qr = sig.qr_found;
            (Some(sig), qr)
        }
        None => (None, false),
    };
    // 2. Текст с учётом QR.
    let mut score = text_detector.score_story(caption, has_url, qr);
    // 3. Визуальный буст.
    if let Some(sig) = &sig_opt {
        let (boost, reasons) = visual_ad_boost(sig);
        score.apply_visual_boost(boost, reasons);
    }
    // 4. CLIP-лайт (второе мнение) + ONNX (если модель настроена).
    if clip_enabled {
        if let Some(sig) = &sig_opt {
            let (boost, reasons) = clip_lite_boost(caption, sig);
            score.apply_clip_boost(boost, reasons);
        } else {
            let (boost, reasons) = clip_lite_boost(caption, &VisualSignals::default());
            score.apply_clip_boost(boost, reasons);
        }
    }
    score
}

/// Полный скоринг сторис: база + ONNX при наличии (через SQLite-кэш).
/// Возвращает готовый `AdScore`. Используется UI для карточек сторис.
pub fn score_story_full(
    text_detector: &impl AdDetector,
    caption: &str,
    has_url: bool,
    image: Option<&image::DynamicImage>,
    clip_enabled: bool,
    onnx_model: &str,
    onnx_enabled: bool,
    onnx_input: &str,
    onnx_provider: &str,
    onnx_db: Option<&crate::db::Db>,
) -> AdScore {
    let mut score = score_story_base(text_detector, caption, has_url, image, clip_enabled);
    if let Some(img) = image {
        if let Some((boost, reasons)) = try_onnx_boost(
            onnx_db,
            onnx_model,
            onnx_enabled,
            img,
            onnx_input,
            onnx_provider,
        ) {
            score.apply_clip_boost(boost, reasons);
        }
    }
    score
}
