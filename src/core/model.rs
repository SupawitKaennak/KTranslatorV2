use serde::{Deserialize, Serialize};

use crate::core::{
    ports::OcrTextLine,
    types::{LanguageTag, Rect, RegionId},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionSlot {
    pub id: RegionId,
    pub display_id: u32,
    pub enabled: bool,
    pub show_frame: bool,
    pub rect: Option<Rect>,
    pub source_lang: Option<LanguageTag>, // None = auto
    pub target_lang: LanguageTag,
    #[serde(skip)]
    pub stable_hash: u64,
    #[serde(skip)]
    pub stable_since_ms: u64,
    pub refresh_ms: u64,
    pub last_ocr_text: String,
    pub last_translation: String,
    /// Per-line OCR results with bounding boxes (image-pixel coordinates).
    /// Used by the positional overlay to place translated text at the right position.
    #[serde(skip)]
    pub last_ocr_lines: Vec<OcrTextLine>,
    /// Translation split by newline, matched index-for-index with last_ocr_lines.
    #[serde(skip)]
    pub last_trans_lines: Vec<String>,
    pub pending_text: String,
    pub next_tick_at_ms: u64,
    pub translate_backoff_ms: u64,
    pub translate_next_try_at_ms: u64,
    pub popup_open: bool,
    pub overlay_mode: bool,
    #[serde(skip)]
    pub language_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppModel {
    pub running: bool,
    pub slots: Vec<RegionSlot>,
}

impl AppModel {
    pub fn new_default() -> Self {
        let mut slots = Vec::new();
        slots.push(RegionSlot {
            id: RegionId(0),
            display_id: 0,
            enabled: false,
            show_frame: false,
            rect: None,
            source_lang: None,
            target_lang: LanguageTag("en".to_string()),
            stable_hash: 0,
            stable_since_ms: 0,
            refresh_ms: 5000,
            last_ocr_text: String::new(),
            last_translation: String::new(),
            last_ocr_lines: Vec::new(),
            last_trans_lines: Vec::new(),
            pending_text: String::new(),
            next_tick_at_ms: 0,
            translate_backoff_ms: 0,
            translate_next_try_at_ms: 0,
            popup_open: false,
            overlay_mode: false,
            language_version: 0,
        });
        Self {
            running: false,
            slots,
        }
    }
    pub fn add_slot(&mut self) -> usize {
        let new_idx = self.slots.len();
        self.slots.push(RegionSlot {
            id: RegionId(new_idx),
            display_id: 0,
            enabled: false,
            show_frame: false,
            rect: None,
            source_lang: None,
            target_lang: LanguageTag("en".to_string()),
            stable_hash: 0,
            stable_since_ms: 0,
            refresh_ms: 5000,
            last_ocr_text: String::new(),
            last_translation: String::new(),
            last_ocr_lines: Vec::new(),
            last_trans_lines: Vec::new(),
            pending_text: String::new(),
            next_tick_at_ms: 0,
            translate_backoff_ms: 0,
            translate_next_try_at_ms: 0,
            popup_open: false,
            overlay_mode: false,
            language_version: 0,
        });
        new_idx
    }
}

