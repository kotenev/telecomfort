//! Перцептивный хэш (pHash, DCT) для поиска визуальных дублей.
//!
//! sha256 ловит только бит-в-бит одинаковые файлы, а один и тот же мем,
//! пересохранённый Telegram с другим качеством/размером, — уже нет.
//! pHash: grayscale → 32x32 → DCT-II → верхние 8x8 частот (без DC) →
//! 64 бита (коэффициент выше среднего = 1). Расстояние Хэмминга ≤
//! `MAX_HAMMING` считаем «похож» (ресайз/рекомпресс/лёгкий кроп).
//!
//! Чистый Rust без зависимостей (DCT наивный O(N³) на 32x32 — микросекунды).
//! Хранится в SQLite (`downloads.phash`), ищется через BK-дерево
//! (`ai::bktree::BkTree`, `Db::find_near_phash`): на тысячах строк так же
//! быстро, как линейный скан, на десятках тысяч — прунинг по неравенству
//! треугольника вместо полного перебора.

/// Максимальное расстояние Хэмминга для «похожего» (из 64 бит).
pub const MAX_HAMMING: u32 = 6;

/// pHash изображения (64 бита). Детерминирован для одних пикселей.
pub fn phash(img: &image::DynamicImage) -> u64 {
    let gray = img.to_luma8();
    let small = image::imageops::resize(&gray, 32, 32, image::imageops::FilterType::Triangle);
    // Вход DCT: яркость 0..1.
    let mut input = [[0.0f64; 32]; 32];
    for (y, row) in input.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            *px = small.get_pixel(x as u32, y as u32)[0] as f64 / 255.0;
        }
    }
    let dct = dct_2d(&input);
    // Верхние 8x8 (низкие частоты), среднее без DC [0][0].
    let mut sum = 0.0f64;
    for row in dct.iter().take(8) {
        for &v in row.iter().take(8) {
            sum += v;
        }
    }
    let avg = (sum - dct[0][0]) / 63.0;
    let mut hash = 0u64;
    for row in dct.iter().take(8) {
        for &v in row.iter().take(8) {
            hash = (hash << 1) | (v > avg) as u64;
        }
    }
    hash
}

/// Расстояние Хэмминга между хэшами (0 = идентичны перцептивно).
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Похожи ли изображения (расстояние ≤ `MAX_HAMMING`).
pub fn is_near_duplicate(a: u64, b: u64) -> bool {
    hamming(a, b) <= MAX_HAMMING
}

/// 2D DCT-II через два прохода 1D (строки, затем столбцы).
fn dct_2d(input: &[[f64; 32]; 32]) -> [[f64; 32]; 32] {
    const N: usize = 32;
    // Предвычисляем косинусы: cos((2x+1)*u*π/2N).
    let mut cos = [[0.0f64; N]; N];
    for (u, row) in cos.iter_mut().enumerate() {
        for (x, slot) in row.iter_mut().enumerate() {
            *slot = (((2 * x + 1) * u) as f64 * std::f64::consts::PI / (2 * N) as f64).cos();
        }
    }
    let norm = |u: usize| {
        if u == 0 {
            (1.0 / N as f64).sqrt()
        } else {
            (2.0 / N as f64).sqrt()
        }
    };
    // По строкам.
    let mut tmp = [[0.0f64; N]; N];
    for (y, tmp_row) in tmp.iter_mut().enumerate() {
        for (u, slot) in tmp_row.iter_mut().enumerate() {
            let mut s = 0.0f64;
            for (x, v) in input[y].iter().enumerate() {
                s += v * cos[u][x];
            }
            *slot = s * norm(u);
        }
    }
    // По столбцам.
    let mut out = [[0.0f64; N]; N];
    for (v, out_row) in out.iter_mut().enumerate() {
        for (u, slot) in out_row.iter_mut().enumerate() {
            let mut s = 0.0f64;
            for y in 0..N {
                s += tmp[y][u] * cos[v][y];
            }
            *slot = s * norm(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GenericImageView};

    fn solid(w: u32, h: u32, r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([r, g, b])))
    }

    /// Половинки: левая тёмная, правая светлая + диагональ для текстуры.
    fn halves(w: u32, h: u32, left: [u8; 3], right: [u8; 3]) -> DynamicImage {
        let mut img = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let c = if x < w / 2 { left } else { right };
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
        // Диагональ — высокочастотная деталь, устойчивая к ресайзу.
        for i in 0..w.min(h) {
            img.put_pixel(i, i, image::Rgb([200, 30, 30]));
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn hamming_basics() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(u64::MAX, 0), 64);
        assert_eq!(hamming(0b1010, 0b1000), 1);
    }

    #[test]
    fn identical_images_zero_distance() {
        let a = halves(128, 128, [20, 20, 20], [230, 230, 230]);
        let b = halves(128, 128, [20, 20, 20], [230, 230, 230]);
        assert_eq!(hamming(phash(&a), phash(&b)), 0);
    }

    #[test]
    fn resize_and_recompress_are_near() {
        let a = halves(256, 256, [20, 20, 20], [230, 230, 230]);
        // Даунскейл + JPEG-рекомпресс (как пересылка через Telegram).
        let small = a.resize(90, 90, image::imageops::FilterType::Triangle);
        let mut buf = Vec::new();
        {
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60);
            let rgb = small.to_rgb8();
            enc.encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        }
        let b = image::load_from_memory(&buf).unwrap();
        let d = hamming(phash(&a), phash(&b));
        assert!(d <= MAX_HAMMING, "distance={d}");
        assert!(is_near_duplicate(phash(&a), phash(&b)));
    }

    #[test]
    fn different_images_are_far() {
        let a = halves(128, 128, [20, 20, 20], [230, 230, 230]);
        let b = halves(128, 128, [230, 230, 230], [20, 20, 20]); // инверсия
        let d = hamming(phash(&a), phash(&b));
        assert!(d > MAX_HAMMING, "distance={d}");
        assert!(!is_near_duplicate(phash(&a), phash(&b)));
        // Солид vs текстура тоже далеко.
        let c = solid(128, 128, 128, 128, 128);
        assert!(!is_near_duplicate(phash(&a), phash(&c)));
    }

    #[test]
    fn brightness_shift_is_near() {
        // pHash почти нечувствителен к яркости (DC исключён из среднего,
        // но влияет на AC слабо).
        let a = halves(128, 128, [40, 40, 40], [210, 210, 210]);
        let b = halves(128, 128, [60, 60, 60], [230, 230, 230]);
        assert!(is_near_duplicate(phash(&a), phash(&b)));
    }
}
