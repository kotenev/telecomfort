//! Замыливание рекламных зон (только реклама, остальное фото — чёткое).

use super::RelRect;
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};

/// Заблюрить указанные относительные зоны. `sigma` — сила блюра.
pub fn blur_regions(img: &DynamicImage, regions: &[RelRect], sigma: f32) -> DynamicImage {
    if regions.is_empty() {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let mut base = img.to_rgba8();

    // Один общий блюр кадра, затем копируем заблюренные куски в резкий оригинал.
    // Так блюр не «течёт» за границы сильнее, чем нужно, и работает быстро.
    let blurred = image::imageops::blur(&base, sigma.clamp(0.5, 25.0));

    for r in regions {
        let x0 = ((r.x.clamp(0.0, 1.0) * w as f32) as u32).min(w.saturating_sub(1));
        let y0 = ((r.y.clamp(0.0, 1.0) * h as f32) as u32).min(h.saturating_sub(1));
        let x1 = (((r.x + r.w).clamp(0.0, 1.0) * w as f32) as u32).min(w);
        let y1 = (((r.y + r.h).clamp(0.0, 1.0) * h as f32) as u32).min(h);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                base.put_pixel(x, y, *blurred.get_pixel(x, y));
            }
        }
        // Тонкая рамка-предупреждение по границе зоны
        draw_rect_border(&mut base, x0, y0, x1, y1, Rgba([255, 90, 90, 220]));
    }

    DynamicImage::ImageRgba8(base)
}

/// Заблюрить всё фото целиком (режим «скрыть сторис полностью»).
pub fn blur_full(img: &DynamicImage, sigma: f32) -> DynamicImage {
    DynamicImage::ImageRgba8(image::imageops::blur(
        &img.to_rgba8(),
        sigma.clamp(0.5, 25.0),
    ))
}

fn draw_rect_border(
    img: &mut ImageBuffer<Rgba<u8>, Vec<u8>>,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    color: Rgba<u8>,
) {
    let (w, h) = (img.width(), img.height());
    let t = 3u32;
    for dy in 0..t {
        for x in x0..x1 {
            if y0 + dy < h {
                img.put_pixel(x, y0 + dy, color);
            }
            if y1 > dy && y1 - 1 - dy >= y0 && y1 - 1 - dy < h {
                img.put_pixel(x, y1 - 1 - dy, color);
            }
        }
    }
    for dx in 0..t {
        for y in y0..y1 {
            if x0 + dx < w {
                img.put_pixel(x0 + dx, y, color);
            }
            if x1 > dx && x1 - 1 - dx >= x0 && x1 - 1 - dx < w {
                img.put_pixel(x1 - 1 - dx, y, color);
            }
        }
    }
}

/// Эвристика «есть ли QR»: ищем характерный finder-паттерн —
/// тёмные квадратные контуры. Грубо, но работает как триггер для детектора
/// без тяжёлых ML-зависимостей. Возвращает true, если найден кандидат.
pub fn looks_like_qr(img: &DynamicImage) -> bool {
    // Даунскейлим до 128px, бинаризуем, ищем ≥2 квадратных контуров в углах.
    let gray = img.to_luma8();
    let small = image::imageops::resize(&gray, 128, 128, image::imageops::FilterType::Nearest);
    let mut dark = 0u32;
    for p in small.pixels() {
        if p[0] < 110 {
            dark += 1;
        }
    }
    let total = (128 * 128) as u32;
    let ratio = dark as f32 / total as f32;
    // QR обычно даёт 15–45% тёмных пикселей + высокую контрастность
    if !(0.12..0.55).contains(&ratio) {
        return false;
    }
    // Проверяем контрастные углы 32x32: в QR finder-паттерны дают плотные тёмные рамки.
    let corner_density = [
        corner_dark(&small, 0, 0),
        corner_dark(&small, 96, 0),
        corner_dark(&small, 0, 96),
    ];
    corner_density.iter().filter(|d| **d > 0.25).count() >= 2
}

fn corner_dark(small: &image::GrayImage, ox: u32, oy: u32) -> f32 {
    let mut dark = 0u32;
    let mut total = 0u32;
    for y in oy..(oy + 32).min(128) {
        for x in ox..(ox + 32).min(128) {
            total += 1;
            if small.get_pixel(x, y)[0] < 110 {
                dark += 1;
            }
        }
    }
    dark as f32 / total.max(1) as f32
}
