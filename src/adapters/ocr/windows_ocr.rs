use anyhow::{Context, Result};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

use crate::core::{
    ports::{FrameRgba, OcrEngine as OcrEngineTrait, OcrTextLine},
    types::LanguageTag,
};

pub struct WindowsOcr;

impl WindowsOcr {
    pub fn new() -> Self {
        Self
    }

    /// Build the OcrEngine for the requested language.
    fn make_engine(lang_hint: Option<&LanguageTag>) -> Result<OcrEngine> {
        if let Some(hint) = lang_hint {
            let tag = &hint.0;
            if let Ok(lang) = windows::Globalization::Language::CreateLanguage(&tag.into()) {
                return OcrEngine::TryCreateFromLanguage(&lang)
                    .context("Failed to create OCR engine for hinted language");
            }
        }
        OcrEngine::TryCreateFromUserProfileLanguages()
            .context("Failed to create default OCR engine")
    }

    /// Convert FrameRgba (RGBA8) to a SoftwareBitmap (Bgra8) suitable for Windows OCR.
    fn to_software_bitmap(frame: &FrameRgba) -> Result<SoftwareBitmap> {
        let mut bgra: Vec<u8> = Vec::with_capacity(frame.data.len());
        for chunk in frame.data.chunks_exact(4) {
            bgra.push(chunk[2]); // B
            bgra.push(chunk[1]); // G
            bgra.push(chunk[0]); // R
            bgra.push(chunk[3]); // A
        }
        let writer = DataWriter::new()?;
        writer.WriteBytes(&bgra)?;
        let buffer = writer.DetachBuffer()?;
        SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            frame.width as i32,
            frame.height as i32,
        )
        .context("create SoftwareBitmap")
    }

    /// Run Windows OCR and return one `OcrTextLine` per recognised line,
    /// each carrying its bounding box in image-pixel coordinates.
    pub fn recognize_lines(
        &self,
        frame: &FrameRgba,
        lang_hint: Option<&LanguageTag>,
    ) -> Result<Vec<OcrTextLine>> {
        let engine = Self::make_engine(lang_hint)?;
        let bitmap = Self::to_software_bitmap(frame)?;

        let operation = engine.RecognizeAsync(&bitmap)?;
        while operation.Status()?.0 == 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let result = operation.GetResults().context("OCR recognition failed")?;

        let lines = result.Lines()?;
        let count = lines.Size()?;
        let mut out = Vec::with_capacity(count as usize);

        for idx in 0..count {
            let line = lines.GetAt(idx)?;
            let text = line.Text()?.to_string();
            if text.trim().is_empty() {
                continue;
            }

            // Compute the bounding rect as the union of all word bounding rects.
            let words = line.Words()?;
            let word_count = words.Size()?;
            let mut min_x = f32::MAX;
            let mut min_y = f32::MAX;
            let mut max_x = f32::MIN;
            let mut max_y = f32::MIN;

            for wi in 0..word_count {
                let word = words.GetAt(wi)?;
                let r = word.BoundingRect()?;
                if r.X < min_x { min_x = r.X; }
                if r.Y < min_y { min_y = r.Y; }
                if r.X + r.Width  > max_x { max_x = r.X + r.Width; }
                if r.Y + r.Height > max_y { max_y = r.Y + r.Height; }
            }

            if min_x < f32::MAX {
                out.push(OcrTextLine {
                    text,
                    x: min_x,
                    y: min_y,
                    w: max_x - min_x,
                    h: max_y - min_y,
                });
            }
        }
        Ok(out)
    }
}

impl OcrEngineTrait for WindowsOcr {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String> {
        let lines = self.recognize_lines(frame, lang_hint)?;
        Ok(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"))
    }
}
