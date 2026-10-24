//! CLIP-лайт: двухбашенная оценка «текст ↔ картинка» без ML-зависимостей.
//!
//! Идея CLIP: текст и изображение проецируются в общее пространство,
//! близость к прототипу «реклама» даёт скор. Здесь эмбеддинги —
//! детерминированные эвристические векторы (32 dim):
//! - dims 0..16: группы промо-лексики (скидки/призывы/деньги/...),
//! - dims 16..20: структурные флаги (url/username/промокод/телефон/эмодзи-капс),
//! - dims 20..26: визуальные сигналы (палитра/контраст/плашки/QR),
//! - dims 26..32: зарезервировано под выход ONNX-модели (`ai/onnx.rs`, фича `ort`).
//!
//! Возвращает буст 0..0.25. Это «второе мнение» поверх heuristic+visual:
//! итоговый скор = text + visual + clip_lite (+ onnx при наличии модели).

use super::visual::VisualSignals;

const DIM: usize = 32;

/// Группы промо-лексики (по 1 dim на группу).
const TOKEN_GROUPS: &[&[&str]] = &[
    &["скидк", "распродаж", "sale", "-50", "%", "дешев"],
    &[
        "подпишись",
        "подписывайся",
        "переходи",
        "жми",
        "swipe",
        "свайп",
    ],
    &["розыгрыш", "giveaway", "подарок", "🎁", "приз"],
    &[
        "казино",
        "casino",
        "ставк",
        "бонус",
        "фриспин",
        "🎰",
        "promocode",
    ],
    &[
        "заработок",
        "доход",
        "трейдинг",
        "крипт",
        "forex",
        "форекс",
        "инвест",
        "💰",
    ],
    &[
        "только сегодня",
        "успей",
        "ограничен",
        "срочно",
        "последний",
    ],
    &["бесплатно", "free", "даром"],
    &["заказ", "доставка", "под ключ", "каталог", "🍔", "🚀"],
    &["промокод", "promo"],
    &["официальн", "дилер", "магазин"],
    &["https", "t.me", "link", "taplink", "bit.ly"],
    &["канал", "подборк", "подписк"],
    &["деньги", "рубл", "usd", "$", "€", "₽", "цена"],
    &["новинк", "акция", "предложен"],
    &["регистрац", "забирай", "переходи в"],
    &["удача", "выигрыш", "джекпот", "🏆"],
];

/// Текстовый эмбеддинг 32 dim.
pub fn embed_text(caption: &str) -> [f32; DIM] {
    let mut v = [0.0f32; DIM];
    let lower = caption.to_lowercase();
    for (i, group) in TOKEN_GROUPS.iter().enumerate().take(16) {
        let mut hit = 0.0f32;
        for tok in *group {
            if lower.contains(tok) {
                hit += 1.0;
            }
        }
        v[i] = hit.min(3.0) / 3.0;
    }
    // Структурные флаги 16..20
    v[16] = flag(&lower, &["http", "t.me", "bit.ly", "taplink"]);
    v[17] = if lower.contains('@') { 1.0 } else { 0.0 };
    v[18] = flag(&lower, &["промокод", "promo"]);
    v[19] = flag(&lower, &["+", "8-9", "8 (9"]) * 0.5 + emoji_caps_flag(caption);
    // 20..26 заполняет embed_image_part, 26..32 резерв.
    v
}

fn flag(lower: &str, toks: &[&str]) -> f32 {
    if toks.iter().any(|t| lower.contains(t)) {
        1.0
    } else {
        0.0
    }
}

fn emoji_caps_flag(caption: &str) -> f32 {
    const EMOJI: &[char] = &[
        '🔥', '💰', '🎁', '💸', '🤑', '⚡', '🚀', '💯', '🎉', '🏆', '🎰',
    ];
    let e: usize = caption.chars().filter(|c| EMOJI.contains(c)).count();
    let letters: Vec<char> = caption.chars().filter(|c| c.is_alphabetic()).collect();
    let caps = if letters.len() >= 12 {
        let up = letters.iter().filter(|c| c.is_uppercase()).count();
        if up as f32 / letters.len() as f32 > 0.6 {
            0.5
        } else {
            0.0
        }
    } else {
        0.0
    };
    ((e.min(4) as f32) / 4.0 * 0.7 + caps).min(1.0)
}

/// Визуальный эмбеддинг: дописывает dims 20..26 в общий вектор.
pub fn embed_joint(caption: &str, sig: &VisualSignals) -> [f32; DIM] {
    let mut v = embed_text(caption);
    v[20] = (sig.promo_palette_ratio * 2.5).min(1.0);
    v[21] = sig.edge_density.min(1.0);
    v[22] = sig.bright_ratio.min(1.0);
    v[23] = sig.band_bottom.min(1.0);
    v[24] = sig.band_top.min(1.0);
    v[25] = if sig.qr_found { 1.0 } else { 0.0 };
    v
}

/// Косинусная близость к «рекламному» прототипу → буст 0..0.25.
pub fn clip_lite_boost(caption: &str, sig: &VisualSignals) -> (f32, Vec<String>) {
    let v = embed_joint(caption, sig);
    // Прототип «реклама»: промо-лексика + структура + визуал активны.
    let mut proto = [0.0f32; DIM];
    for i in 0..16 {
        proto[i] = 0.7;
    }
    for i in 16..20 {
        proto[i] = 0.8;
    }
    proto[20] = 0.7;
    proto[21] = 0.6;
    proto[22] = 0.5;
    proto[23] = 0.7;
    proto[24] = 0.5;
    proto[25] = 0.9;
    let sim = cosine(&v, &proto);
    // Нейтральный текст даёт sim ~0.1-0.3, реклама ~0.55-0.9.
    let boost = ((sim - 0.35) * 0.55).clamp(0.0, 0.25);
    let mut reasons = Vec::new();
    if boost >= 0.12 {
        reasons.push(format!(
            "CLIP-лайт: близко к прототипу рекламы ({:.0}%)",
            sim * 100.0
        ));
    }
    (boost, reasons)
}

fn cosine(a: &[f32; DIM], b: &[f32; DIM]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..DIM {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-9 || nb <= 1e-9 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral_sig() -> VisualSignals {
        VisualSignals::default()
    }

    fn ad_sig() -> VisualSignals {
        VisualSignals {
            promo_palette_ratio: 0.35,
            edge_density: 0.5,
            bright_ratio: 0.5,
            qr_found: true,
            band_top: 0.7,
            band_bottom: 0.7,
        }
    }

    #[test]
    fn neutral_low_ad_high() {
        let (n, _) = clip_lite_boost("Гуляли в парке, хорошая погода", &neutral_sig());
        let (a, reasons) = clip_lite_boost(
            "СКИДКА -50% только сегодня! Переходи по ссылке https://shop.example, промокод SALE10",
            &ad_sig(),
        );
        assert!(a > n, "ad={a} neutral={n}");
        assert!(n < 0.08, "neutral boost too high: {n}");
        assert!(a > 0.10, "ad boost too low: {a}");
        assert!(!reasons.is_empty());
    }

    #[test]
    fn embeddings_bounded() {
        let v = embed_joint("test @shop https://x", &ad_sig());
        for x in v {
            assert!(x >= 0.0 && x <= 1.5, "{x}");
        }
    }
}
