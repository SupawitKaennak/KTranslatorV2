use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranslationProvider {
    Gemini,
    Groq,
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcrEngineType {
    Windows,
    Paddle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub provider: TranslationProvider,
    pub ocr_engine: OcrEngineType,
    pub paddle_ocr_path: String,
    pub gemini_api_key: String,
    pub gemini_model: String,
    pub groq_api_key: String,
    pub groq_model: String,
    pub ollama_url: String,
    pub ollama_model: String,
    pub dark_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: TranslationProvider::Gemini,
            ocr_engine: OcrEngineType::Windows,
            paddle_ocr_path: String::new(),
            gemini_api_key: String::new(),
            gemini_model: "gemini-2.0-flash".to_string(),
            groq_api_key: String::new(),
            groq_model: "llama-3.3-70b-versatile".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            ollama_model: "llama3.2:1b".to_string(),
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

