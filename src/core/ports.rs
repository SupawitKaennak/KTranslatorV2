use anyhow::Result;

use crate::core::types::{LanguageTag, Rect};

#[derive(Debug, Clone)]
pub struct FrameRgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA8
}

/// One line of OCR-recognised text together with its bounding box in
/// image-pixel coordinates (origin = top-left of the captured frame).
/// Used by the positional overlay to render translated text at the same
/// position as the original source text.
#[derive(Debug, Clone, Default)]
pub struct OcrTextLine {
    pub text: String,
    pub x: f32,
    pub y: f32,
    #[allow(dead_code)] // kept for future text-wrapping / overflow detection
    pub w: f32,
    pub h: f32,
}

pub trait FrameSource: Send + Sync {
    fn capture_rect(&self, rect: Rect, display_id: u32) -> Result<FrameRgba>;
}

#[allow(dead_code)] // trait contract; used by GeminiOcr and may be called directly in future
pub trait OcrEngine: Send + Sync {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String>;
    fn recognize_lines(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<Vec<OcrTextLine>>;
}

pub trait Translator: Send + Sync {
    fn translate(
        &self,
        text: &str,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<String>;
}

