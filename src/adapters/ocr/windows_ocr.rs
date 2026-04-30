use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};
use std::collections::HashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use windows::Graphics::Imaging::SoftwareBitmap;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

use crate::core::types::LanguageTag;
use crate::core::ports::{FrameRgba, OcrTextLine};

pub struct WindowsOcr {
    engines: Arc<Mutex<HashMap<String, OcrEngine>>>,
}

impl WindowsOcr {
    pub fn new() -> Self {
        Self {
            engines: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn make_engine(lang_hint: Option<&LanguageTag>) -> Result<OcrEngine> {
        if let Some(tag) = lang_hint {
            let win_tag = windows::Globalization::Language::CreateLanguage(&windows::core::HSTRING::from(&tag.0))?;
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&win_tag) {
                return Ok(engine);
            }
        }
        Ok(OcrEngine::TryCreateFromUserProfileLanguages()?)
    }

    fn to_software_bitmap(frame: &FrameRgba) -> Result<SoftwareBitmap> {
        let bitmap = SoftwareBitmap::Create(
            windows::Graphics::Imaging::BitmapPixelFormat::Rgba8,
            frame.width as i32,
            frame.height as i32,
        )?;

        let dw = DataWriter::new()?;
        dw.WriteBytes(&frame.data)?;
        let buffer = dw.DetachBuffer()?;

        bitmap.CopyFromBuffer(&buffer)?;
        Ok(bitmap)
    }

    fn preprocess(frame: &FrameRgba) -> (FrameRgba, f32) {
        let Some(img) = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(frame.width, frame.height, frame.data.clone()) else {
            return (frame.clone(), 1.0);
        };

        let dynamic = image::DynamicImage::ImageRgba8(img);
        let sharpened = dynamic.unsharpen(1.0, 15);
        let gray_img = sharpened.to_luma8();

        let (processed_img, final_scale) = if frame.height < 1200 {
            let scale = 3.0;
            let new_w = (frame.width as f32 * scale) as u32;
            let new_h = (frame.height as f32 * scale) as u32;
            let resized = image::imageops::resize(&gray_img, new_w, new_h, image::imageops::FilterType::CatmullRom);
            (resized, scale)
        } else {
            (gray_img, 1.0)
        };
        let scale = final_scale;

        let mut final_img: image::ImageBuffer<image::Luma<u8>, Vec<u8>> = processed_img;
        
        // Dynamic Contrast Stretching instead of hard thresholding
        // This is much safer for manga with screentones/grey backgrounds.
        let mut min_v = 255u8;
        let mut max_v = 0u8;
        for pixel in final_img.pixels() {
            let v = pixel.0[0];
            if v < min_v { min_v = v; }
            if v > max_v { max_v = v; }
        }
        
        if max_v > min_v {
            let range = (max_v - min_v) as f32;
            for pixel in final_img.pixels_mut() {
                let v = pixel.0[0];
                let normalized = ((v - min_v) as f32 / range * 255.0) as u8;
                pixel.0[0] = normalized;
            }
        }

        let final_rgba = image::DynamicImage::ImageLuma8(final_img).to_rgba8();
        
        (
            FrameRgba {
                width: final_rgba.width(),
                height: final_rgba.height(),
                data: final_rgba.into_raw(),
            },
            scale,
        )
    }

    pub fn recognize_lines(
        &self,
        frame: &FrameRgba,
        lang_hint: Option<&LanguageTag>,
    ) -> Result<Vec<OcrTextLine>> {
        let lang_key = lang_hint
            .map(|l| l.0.clone())
            .unwrap_or_else(|| "default".to_string());

        let engine = {
            let mut cache = self.engines.lock();
            if !cache.contains_key(&lang_key) {
                cache.insert(lang_key.clone(), Self::make_engine(lang_hint)?);
            }
            cache.get(&lang_key).unwrap().clone()
        };

        let (processed_frame, scale) = Self::preprocess(frame);
        let bitmap = Self::to_software_bitmap(&processed_frame)?;

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
            
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }

            // OcrLine doesn't have a direct Rect property. 
            // We must calculate the bounding box from its words.
            let words = line.Words()?;
            let word_count = words.Size()?;
            if word_count == 0 { continue; }

            let mut min_x = f32::MAX;
            let mut min_y = f32::MAX;
            let mut max_x = f32::MIN;
            let mut max_y = f32::MIN;

            for w_idx in 0..word_count {
                let word = words.GetAt(w_idx)?;
                let rect = word.BoundingRect()?;
                min_x = min_x.min(rect.X);
                min_y = min_y.min(rect.Y);
                max_x = max_x.max(rect.X + rect.Width);
                max_y = max_y.max(rect.Y + rect.Height);
            }
            
            out.push(OcrTextLine {
                text,
                x: min_x / scale,
                y: min_y / scale,
                w: (max_x - min_x) / scale,
                h: (max_y - min_y) / scale,
            });
        }

        Ok(out)
    }

    pub fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String> {
        let lines = self.recognize_lines(frame, lang_hint)?;
        let full_text = lines.iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(full_text)
    }
}
