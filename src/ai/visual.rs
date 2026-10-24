//! Визуальный ИИ-скоринг сторис без тяжёлых ML-зависимостей.
//!
//! Идея: рекламные кадры отличаются от личных фото статистически:
//! - кричащая палитра (красный/оранжевый/жёлтый, высокая насыщенность),
//! - высокая локальная контрастность (текст, плашки, цены),
//! - плотные горизонтальные «текстовые полосы» сверху/снизу,
//! - QR-кандидат (см. `blur::looks_like_qr`).
//!
//! Это не ONNX/CLIP, а лёгкий CV-скоринг 0..0.45, который складывается
//! с текстовым скором эвристик. Точка расширения: замените
//! `analyze_image` на вызов ONNX-модели за фичей `ort` —
//! интерфейс (`VisualSignals` → boost) останется тем же.

use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisualSignals {
    /// 0..1 доля «кричащих» пикселей (красный/оранжевый/жёлтый, насыщенные)
    pub promo_palette_ratio: f32,
    /// 0..1 средняя локальная контрастность (текст/плашки)
    pub edge_density: f32,
    /// 0..1 доля ярких пикселей (белые плашки)
    pub bright_ratio: f32,
    /// найден ли QR-кандидат
    pub qr_found: bool,
    /// есть ли плотные горизонтальные полосы сверху/снизу (текст/CTA)
    pub band_top: f32,
    pub band_bottom: f32,
}

/// Быстрый анализ кадра 64x64.
pub fn analyze_image(img: &DynamicImage) -> VisualSignals {
    let small = img.resize_exact(64, 64, image::imageops::FilterType::Triangle);
    let rgb = small.to_rgb8();
    let mut promo = 0u32;
    let mut bright = 0u32;
    let mut total = 0u32;
    // Контраст: считаем дисперсию яркости в окне 3x3 через разности соседей.
    let mut edge_acc = 0u64;
    let mut edge_n = 0u64;
    let luma = |r: u8, g: u8, b: u8| -> u8 {
        ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
    };

    let mut top_band_dark = 0u32;
    let mut top_total = 0u32;
    let mut bottom_band_dark = 0u32;
    let mut bottom_total = 0u32;

    for y in 0..64u32 {
        for x in 0..64u32 {
            let p = rgb.get_pixel(x, y);
            let (r, g, b) = (p[0], p[1], p[2]);
            total += 1;
            let mx = r.max(g).max(b);
            let mn = r.min(g).min(b);
            let sat = mx.saturating_sub(mn);
            // Кричащая палитра: насыщенный + тёплый оттенок (R доминирует).
            let warm = r > 150 && r as i16 - b as i16 > 40 && sat > 70;
            let yellow = r > 170 && g > 150 && b < 110 && sat > 50;
            if warm || yellow {
                promo += 1;
            }
            if mx > 235 {
                bright += 1;
            }
            if x + 1 < 64 {
                let q = rgb.get_pixel(x + 1, y);
                edge_acc += (luma(r, g, b) as i16 - luma(q[0], q[1], q[2]) as i16).abs() as u64;
                edge_n += 1;
            }
            if y + 1 < 64 {
                let q = rgb.get_pixel(x, y + 1);
                edge_acc += (luma(r, g, b) as i16 - luma(q[0], q[1], q[2]) as i16).abs() as u64;
                edge_n += 1;
            }
            // Полосы: верхние 12 строк и нижние 20 строк — ищем тёмный текст
            // на светлом или наоборот (высокий локальный контраст уже учтён,
            // здесь — доля «плашечных» пикселей: очень светлые или очень тёмные).
            let is_flat = mx > 230 || mx < 35;
            if y < 12 {
                top_total += 1;
                if is_flat {
                    top_band_dark += 1;
                }
            }
            if y >= 44 {
                bottom_total += 1;
                if is_flat {
                    bottom_band_dark += 1;
                }
            }
        }
    }

    let qr_found = super::blur::looks_like_qr(img);
    VisualSignals {
        promo_palette_ratio: promo as f32 / total.max(1) as f32,
        edge_density: ((edge_acc as f32 / edge_n.max(1) as f32) / 64.0).clamp(0.0, 1.0),
        bright_ratio: bright as f32 / total.max(1) as f32,
        qr_found,
        band_top: top_band_dark as f32 / top_total.max(1) as f32,
        band_bottom: bottom_band_dark as f32 / bottom_total.max(1) as f32,
    }
}

/// Перевод визуальных сигналов в буст 0..0.45 + причины.
pub fn visual_ad_boost(s: &VisualSignals) -> (f32, Vec<String>) {
    let mut boost = 0.0f32;
    let mut reasons = Vec::new();
    if s.qr_found {
        boost += 0.18;
        reasons.push("QR-код на кадре (визуально)".to_string());
    }
    if s.promo_palette_ratio > 0.18 {
        boost += 0.10;
        reasons.push(format!(
            "кричащая палитра {:.0}%",
            s.promo_palette_ratio * 100.0
        ));
    } else if s.promo_palette_ratio > 0.10 {
        boost += 0.05;
    }
    if s.edge_density > 0.35 {
        boost += 0.08;
        reasons.push("плотный текст/плашки на кадре".to_string());
    } else if s.edge_density > 0.25 {
        boost += 0.04;
    }
    if s.band_bottom > 0.55 || s.band_top > 0.55 {
        boost += 0.07;
        reasons.push("рекламная вёрстка: плашка сверху/снизу".to_string());
    }
    if s.bright_ratio > 0.45 && s.edge_density > 0.25 {
        boost += 0.05;
        reasons.push("белая подложка под текст (ценник)".to_string());
    }
    (boost.clamp(0.0, 0.45), reasons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::blur::{blur_full, blur_regions};
    use crate::ai::RelRect;

    fn solid(r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(128, 128, image::Rgb([r, g, b])))
    }

    #[test]
    fn calm_photo_low_boost() {
        // Спокойное фото: серо-зелёный, без плашек.
        let s = analyze_image(&solid(90, 120, 110));
        let (boost, _) = visual_ad_boost(&s);
        assert!(boost < 0.12, "boost={boost:?} signals={s:?}");
    }

    #[test]
    fn promo_palette_high_boost() {
        // Кричащий красный фон + белая полоса снизу (эмулируем ценник).
        let mut img = image::RgbImage::from_pixel(128, 128, image::Rgb([230, 40, 30]));
        for y in 90..128 {
            for x in 0..128 {
                img.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
        // Чёрный текст-полоса поверх белого
        for y in 100..108 {
            for x in 10..118 {
                img.put_pixel(x, y, image::Rgb([10, 10, 10]));
            }
        }
        let dynimg = DynamicImage::ImageRgb8(img);
        let s = analyze_image(&dynimg);
        let (boost, _) = visual_ad_boost(&s);
        assert!(boost > 0.10, "boost={boost} signals={s:?}");
    }

    #[test]
    fn qr_boost_present() {
        // QR-паттерн сверху должен давать буст через looks_like_qr или палитру.
        // Проверяем, что функция не паникует и возвращает диапазон.
        let s = VisualSignals {
            qr_found: true,
            ..Default::default()
        };
        let (boost, reasons) = visual_ad_boost(&s);
        assert!(boost >= 0.18);
        assert!(!reasons.is_empty());
    }

    #[test]
    fn blur_regions_only_affects_region() {
        let img = solid(100, 100, 100);
        let (w, h) = img.dimensions();
        assert_eq!((w, h), (128, 128));
        let out = blur_regions(
            &img,
            &[RelRect {
                x: 0.0,
                y: 0.7,
                w: 1.0,
                h: 0.3,
            }],
            6.0,
        );
        // Верх должен остаться резким (пиксель совпадает), низ — заблюрен/обрамлён.
        let top_orig = img.get_pixel(64, 10);
        let top_out = out.get_pixel(64, 10);
        assert_eq!(top_orig, top_out);
        let _ = blur_full(&img, 4.0);
    }
}
