#[cfg(not(windows))]
use anyhow::{Context, Result};
use leptess::{LepTess, tesseract};
use std::sync::Mutex;
use crate::core::{
    ports::{FrameRgba, OcrEngine as OcrEngineTrait, OcrTextLine},
    types::LanguageTag,
};

pub struct TesseractOcr {
    // Tesseract engine is NOT thread-safe, so we wrap it in a Mutex.
    // We also use a simple cache or just one instance.
    api: Mutex<LepTess>,
}

impl TesseractOcr {
    pub fn new(lang: &str) -> Result<Self> {
        let api = LepTess::new(None, lang).context("Failed to initialize Tesseract")?;
        Ok(Self {
            api: Mutex::new(api),
        })
    }

    fn lang_tag_to_tess(tag: Option<&LanguageTag>) -> &str {
        match tag.map(|t| t.0.as_str()) {
            Some("en") => "eng",
            Some("ja") => "jpn",
            Some("th") => "tha",
            Some("zh") => "chi_sim",
            _ => "eng", // default
        }
    }
}

impl OcrEngineTrait for TesseractOcr {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String> {
        let mut api = self.api.lock().unwrap();
        
        // Tesseract needs to know which language to use.
        // Note: Switching languages in Leptess might require re-init if not careful,
        // but for now let's assume the init language is correct.
        
        api.set_image_from_mem(&frame.data, frame.width as i32, frame.height as i32, 4, (frame.width * 4) as i32)
            .context("Failed to set image for Tesseract")?;
            
        let text = api.get_utf8_text().context("Tesseract failed to extract text")?;
        Ok(text)
    }

    fn recognize_lines(&self, _frame: &FrameRgba, _lang_hint: Option<&LanguageTag>) -> Result<Vec<OcrTextLine>> {
        // Implementing recognize_lines for Tesseract is more complex as it requires
        // iterate through ResultIterator to get bounding boxes. 
        // For the first version, let's return a single block or implement a basic version.
        // (We can refine this later to match Windows OCR's line-by-line precision)
        
        let text = self.recognize(_frame, _lang_hint)?;
        if text.trim().is_empty() { return Ok(vec![]); }
        
        Ok(vec![OcrTextLine {
            text,
            x: 0.0, y: 0.0, w: _frame.width as f32, h: _frame.height as f32,
        }])
    }
}
