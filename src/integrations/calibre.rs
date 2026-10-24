//! Интеграция с Calibre: автоимпорт книг + устранение дублей.
//!
//! Использует CLI `calibredb`:
//!   calibredb add --with-library <LIB> <FILE>
//! Перед импортом проверяем дубли:
//!   1) по sha256 (таблица seen_hashes / downloads),
//!   2) по нормализованному title+author через `calibredb list`.

use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

#[derive(Debug)]
pub enum ImportOutcome {
    Imported { book_id: Option<String> },
    Duplicate { reason: String },
    Skipped(String),
}

pub async fn import_book(
    library_path: &str,
    file: &Path,
    delete_after: bool,
) -> Result<ImportOutcome> {
    if library_path.is_empty() {
        return Ok(ImportOutcome::Skipped(
            "не указан путь к библиотеке Calibre".into(),
        ));
    }
    if !Path::new(library_path).exists() {
        return Ok(ImportOutcome::Skipped(format!(
            "библиотека не найдена: {library_path}"
        )));
    }
    // Проверяем наличие calibredb
    let check = Command::new("calibredb").arg("--version").output().await;
    let Ok(check) = check else {
        return Ok(ImportOutcome::Skipped(
            "calibredb не найден в PATH (установите Calibre)".into(),
        ));
    };
    if !check.status.success() {
        return Ok(ImportOutcome::Skipped("calibredb не запускается".into()));
    }

    // Быстрая проверка дубля по имени (нормализованному) через list
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    if !stem.is_empty() {
        if let Ok(dup) = is_duplicate_by_title(library_path, &stem).await {
            if dup {
                return Ok(ImportOutcome::Duplicate {
                    reason: format!("книга «{stem}» уже есть в библиотеке"),
                });
            }
        }
    }

    let out = Command::new("calibredb")
        .arg("add")
        .arg("--with-library")
        .arg(library_path)
        .arg(file)
        .output()
        .await
        .context("run calibredb add")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        // Calibre сам сообщает о дублях — считаем это не ошибкой
        if err.to_lowercase().contains("already") || err.to_lowercase().contains("duplicate") {
            return Ok(ImportOutcome::Duplicate { reason: err });
        }
        anyhow::bail!("calibredb add failed: {err}");
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    tracing::info!("calibre import ok: {}", stdout.trim());

    if delete_after {
        let _ = tokio::fs::remove_file(file).await;
    }
    Ok(ImportOutcome::Imported {
        book_id: parse_added_id(&stdout),
    })
}

async fn is_duplicate_by_title(library_path: &str, stem: &str) -> Result<bool> {
    let norm = crate::utils::normalize_title(stem);
    if norm.len() < 4 {
        return Ok(false);
    }
    let out = Command::new("calibredb")
        .arg("list")
        .arg("--with-library")
        .arg(library_path)
        .arg("--fields")
        .arg("title,authors")
        .arg("--for-machine")
        .output()
        .await
        .context("calibredb list")?;
    if !out.status.success() {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    // Грубая проверка: все значимые слова заголовка встречаются в выводе
    let words: Vec<&str> = norm.split_whitespace().filter(|w| w.len() >= 4).collect();
    if words.is_empty() {
        return Ok(false);
    }
    let hits = words.iter().filter(|w| text.contains(**w)).count();
    Ok(hits * 2 >= words.len() && hits >= 2)
}

fn parse_added_id(stdout: &str) -> Option<String> {
    // Пример: "Added book ids: 42"
    for line in stdout.lines() {
        let l = line.to_lowercase();
        if l.contains("added book") {
            let digits: String = line.chars().filter(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    None
}
