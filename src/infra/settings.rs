use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub gemini_api_key: String,
    pub gemini_model: String,
    pub dark_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gemini_api_key: String::new(),
            gemini_model: "gemini-2.0-flash".to_string(),
            dark_mode: true,
        }
    }
}

fn settings_path() -> Result<PathBuf> {
    let proj = ProjectDirs::from("com", "cursor", "screen_translator")
        .context("ProjectDirs not available")?;
    Ok(proj.config_dir().join("settings.json"))
}

pub fn load_settings() -> Result<Settings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("read settings at {}", path.display()))?;
    let s = serde_json::from_slice(&bytes).context("parse settings.json")?;
    Ok(s)
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(settings).context("serialize settings")?;
    fs::write(&path, bytes).with_context(|| format!("write settings at {}", path.display()))?;
    Ok(())
}

