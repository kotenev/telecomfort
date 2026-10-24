//! Эвристический + «ИИ-лайт» детектор рекламы (RU/EN).
//!
//! Комбинация сигналов:
//! - CTA-фразы («подпишись», «скидка», «промокод», «розыгрыш»...)
//! - URL / @username / телефон / цена / % / «-50%»
//! - Эмодзи-спам (🔥💰🎁...) и КАПС
//! - QR-подсказка от визуального анализа (передаётся снаружи)
//!
//! Веса подобраны так, чтобы обычный личный сторис давал < 0.3,
//! а явная реклама — > 0.7. Порог по умолчанию 0.55 (настраивается).
//! Модуль легко заменить ONNX/CLIP-моделью: реализуйте `AdDetector`.

use super::{AdDetector, AdScore, RelRect};
use regex::Regex;
use std::sync::OnceLock;

static URL_RE: OnceLock<Regex> = OnceLock::new();
static PHONE_RE: OnceLock<Regex> = OnceLock::new();
static PRICE_RE: OnceLock<Regex> = OnceLock::new();
static PROMO_RE: OnceLock<Regex> = OnceLock::new();
static USERNAME_RE: OnceLock<Regex> = OnceLock::new();

fn url_re() -> &'static Regex {
    URL_RE.get_or_init(|| {
        Regex::new(
            r"(?i)(https?://|t\.me/|telegram\.me/|bit\.ly|tinyurl|taplink|linktr\.ee|wa\.me)",
        )
        .unwrap()
    })
}
fn phone_re() -> &'static Regex {
    PHONE_RE.get_or_init(|| Regex::new(r"\+?\d[\d\s\-\(\)]{7,}\d").unwrap())
}
fn price_re() -> &'static Regex {
    PRICE_RE.get_or_init(|| {
        Regex::new(r"(?i)(\d+\s?(₽|руб|usd|\$|€|₸|сом)|скидк|распродаж|sale|-?\d{1,3}\s?%)")
            .unwrap()
    })
}
fn username_re() -> &'static Regex {
    USERNAME_RE.get_or_init(|| Regex::new(r"@[A-Za-z0-9_]{4,}").unwrap())
}
fn promo_re() -> &'static Regex {
    PROMO_RE.get_or_init(|| Regex::new(r"(?i)(промокод|промо-код|promo\s?code)").unwrap())
}

const CTA_PHRASES: &[(&str, f32, &str)] = &[
    ("подпишись", 0.25, "CTA «подпишись»"),
    ("подписывайся", 0.25, "CTA «подписывайся»"),
    ("переходи по ссылке", 0.35, "CTA «переходи по ссылке»"),
    ("переходи в", 0.20, "CTA «переходи»"),
    ("жми на ссылку", 0.30, "CTA «жми на ссылку»"),
    ("swipe up", 0.30, "CTA «swipe up»"),
    ("свайп", 0.20, "CTA «свайп»"),
    ("скидка", 0.25, "слово «скидка»"),
    ("распродажа", 0.25, "слово «распродажа»"),
    ("розыгрыш", 0.30, "слово «розыгрыш»"),
    ("giveaway", 0.30, "слово «giveaway»"),
    ("казино", 0.35, "слово «казино»"),
    ("casino", 0.35, "слово «casino»"),
    ("ставк", 0.25, "слово «ставк*» (беттинг)"),
    ("заработок", 0.25, "слово «заработок»"),
    ("доход от", 0.25, "обещание дохода"),
    ("крипт", 0.20, "слово «крипт*»"),
    ("forex", 0.25, "слово «forex»"),
    ("форекс", 0.25, "слово «форекс»"),
    ("трейдинг", 0.20, "слово «трейдинг»"),
    ("инвест", 0.15, "слово «инвест*»"),
    ("только сегодня", 0.20, "срочность «только сегодня»"),
    ("успей", 0.15, "срочность «успей»"),
    ("ограниченное предложение", 0.25, "срочность предложения"),
    ("бесплатно", 0.12, "слово «бесплатно» (маркетинг)"),
    ("free", 0.10, "слово «free»"),
    ("заказ", 0.10, "слово «заказ»"),
    ("доставка", 0.10, "слово «доставка»"),
    ("под ключ", 0.15, "фраза «под ключ»"),
    ("официальный дилер", 0.25, "фраза «официальный дилер»"),
];

const EMOJI_SPAM: &[char] = &[
    '🔥', '💰', '🎁', '💸', '💵', '🤑', '⚡', '🚀', '💯', '🎉', '🏆',
];

pub struct HeuristicAdDetector;

impl Default for HeuristicAdDetector {
    fn default() -> Self {
        Self
    }
}

impl HeuristicAdDetector {
    pub fn new() -> Self {
        Self
    }
}

impl AdDetector for HeuristicAdDetector {
    fn score_story(&self, caption: &str, has_url_entity: bool, has_qr_hint: bool) -> AdScore {
        let mut score = 0.0f32;
        let mut reasons = Vec::new();
        let mut spans = Vec::new();
        let lower = caption.to_lowercase();

        for (phrase, w, label) in CTA_PHRASES {
            if lower.contains(phrase) {
                score += w;
                reasons.push(label.to_string());
                spans.push((*phrase).to_string());
            }
        }

        if url_re().is_match(caption) || has_url_entity {
            score += 0.35;
            reasons.push("ссылка/URL в сторис".into());
            if let Some(m) = url_re().find(caption) {
                spans.push(m.as_str().to_string());
            }
        }
        if promo_re().is_match(caption) {
            score += 0.30;
            reasons.push("промокод".into());
            spans.push("промокод".into());
        }
        if username_re().is_match(caption) {
            score += 0.15;
            reasons.push("@username (продвижение канала)".into());
        }
        if price_re().is_match(caption) {
            score += 0.25;
            reasons.push("цена/скидка/%".into());
        }
        // Телефон без контекста — слабый сигнал, с CTA — сильный
        if phone_re().is_match(caption) {
            score += 0.15;
            reasons.push("номер телефона".into());
        }
        if has_qr_hint {
            score += 0.40;
            reasons.push("QR-код на изображении".into());
        }

        // Эмодзи-спам
        let emoji_count: usize = caption.chars().filter(|c| EMOJI_SPAM.contains(c)).count();
        if emoji_count >= 3 {
            score += 0.15;
            reasons.push(format!("эмодзи-спам ×{emoji_count}"));
        } else if emoji_count >= 2 && !caption.is_empty() {
            score += 0.07;
        }

        // КАПС: доля заглавных букв среди букв
        let letters: Vec<char> = caption.chars().filter(|c| c.is_alphabetic()).collect();
        if letters.len() >= 12 {
            let upper = letters.iter().filter(|c| c.is_uppercase()).count();
            let ratio = upper as f32 / letters.len() as f32;
            if ratio > 0.6 {
                score += 0.12;
                reasons.push("КАПС в тексте".into());
            }
        }

        // Много строк с эмодзи-префиксом (типичная рекламная вёрстка)
        let lines = caption.lines().count();
        if lines >= 4 && (url_re().is_match(caption) || emoji_count >= 2) {
            score += 0.08;
            reasons.push("рекламная вёрстка (много строк + ссылка/эмодзи)".into());
        }

        score = score.clamp(0.0, 0.98);

        // Зоны блюра: реклама обычно живёт внизу (CTA/ссылка/цена) и вверху (заголовок).
        let mut regions = Vec::new();
        if score > 0.15 {
            let has_bottom_signal =
                url_re().is_match(caption) || price_re().is_match(caption) || has_qr_hint;
            if has_bottom_signal {
                // Нижняя треть — основная рекламная зона
                regions.push(RelRect {
                    x: 0.0,
                    y: 0.62,
                    w: 1.0,
                    h: 0.38,
                });
            }
            if promo_re().is_match(caption) || username_re().is_match(caption) {
                regions.push(RelRect {
                    x: 0.05,
                    y: 0.02,
                    w: 0.90,
                    h: 0.18,
                });
            }
            if regions.is_empty() {
                // Общий случай: мягко блюрим низ
                regions.push(RelRect {
                    x: 0.0,
                    y: 0.70,
                    w: 1.0,
                    h: 0.30,
                });
            }
        }

        AdScore {
            score,
            reasons,
            spans,
            regions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_story_scores_low() {
        let d = HeuristicAdDetector::new();
        let s = d.score_story("Гуляли в парке, хорошая погода ☀️", false, false);
        assert!(s.score < 0.3, "score={}", s.score);
    }

    #[test]
    fn ad_story_scores_high() {
        let d = HeuristicAdDetector::new();
        let s = d.score_story(
            "🔥🔥🔥 СКИДКА -50% только сегодня! Переходи по ссылке https://shop.example, промокод SALE10, успей заказать! @shop",
            true,
            false,
        );
        assert!(s.score > 0.55, "score={}", s.score);
        assert!(!s.regions.is_empty());
    }

    #[test]
    fn qr_hint_boosts() {
        let d = HeuristicAdDetector::new();
        let a = d.score_story("Наш новый каталог", false, false);
        let b = d.score_story("Наш новый каталог", false, true);
        assert!(b.score > a.score);
    }
}
