use anyhow::{Context, Result};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

use crate::core::{
    ports::{FrameRgba, OcrEngine as OcrEngineTrait},
    types::LanguageTag,
};

pub struct WindowsOcr;

impl WindowsOcr {
    pub fn new() -> Self {
        Self
    }
}

impl OcrEngineTrait for WindowsOcr {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String> {
        // 1. Determine language
        let engine = if let Some(hint) = lang_hint {
            let tag = &hint.0;
            if let Ok(lang) = windows::Globalization::Language::CreateLanguage(&tag.into()) {
                OcrEngine::TryCreateFromLanguage(&lang).context("Failed to create OCR engine for hinted language")?
            } else {
                OcrEngine::TryCreateFromUserProfileLanguages().context("Failed to create default OCR engine")?
            }
        } else {
            OcrEngine::TryCreateFromUserProfileLanguages().context("Failed to create default OCR engine")?
        };

        // 2. Convert FrameRgba to SoftwareBitmap (Bgra8)
        let mut bgra_data = Vec::with_capacity(frame.data.len());
        for chunk in frame.data.chunks_exact(4) {
            bgra_data.push(chunk[2]); // B
            bgra_data.push(chunk[1]); // G
            bgra_data.push(chunk[0]); // R
            bgra_data.push(chunk[3]); // A
        }

        let writer = DataWriter::new()?;
        writer.WriteBytes(&bgra_data)?;
        let buffer = writer.DetachBuffer()?;

        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            frame.width as i32,
            frame.height as i32,
        )?;

        // 3. Recognize (Windows Async Operation)
        let operation = engine.RecognizeAsync(&bitmap)?;
        
        // Block until completed by polling status (raw HRESULT/i32 check to avoid missing enum issues).
        // 0 = Started, 1 = Completed, 2 = Canceled, 3 = Error
        while operation.Status()?.0 == 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let result = operation.GetResults().context("OCR recognition failed")?;

        Ok(result.Text()?.to_string())
    }
}
