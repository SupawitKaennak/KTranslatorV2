use anyhow::Result;

use crate::core::types::{LanguageTag, Rect};

#[derive(Debug, Clone)]
pub struct FrameRgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA8
}

pub trait FrameSource: Send + Sync {
    fn capture_rect(&self, rect: Rect, display_id: u32) -> Result<FrameRgba>;
}

pub trait OcrEngine: Send + Sync {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String>;
}

pub trait Translator: Send + Sync {
    fn translate(
        &self,
        text: &str,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<String>;
}

