//! Путь к файлу сессии и helpers.

use std::path::PathBuf;

pub fn default_session_path() -> PathBuf {
    crate::config::default_config_dir().join("grammers.session")
}
