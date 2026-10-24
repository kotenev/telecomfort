pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{t}…")
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{} {}", v as u64, UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Нормализация названия книги/автора для дедупликации.
pub fn normalize_title(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// hex(SHA-256) строки (PIN-код блокировки).
pub fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

pub fn sha256_of_file(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub fn is_ebook(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("epub")
            | Some("fb2")
            | Some("fb2.zip")
            | Some("pdf")
            | Some("mobi")
            | Some("azw3")
            | Some("djvu")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_is_stable_and_64_chars() {
        let a = sha256_hex("1234");
        let b = sha256_hex("1234");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, sha256_hex("1235"));
        // Известный вектор: SHA-256("").
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
