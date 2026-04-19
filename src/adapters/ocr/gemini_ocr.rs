use anyhow::{bail, Context, Result};
use base64::Engine;
use image::ImageBuffer;
use image::Rgba;
use reqwest::blocking::Client;
use std::io::Cursor;

use crate::core::{
    ports::{FrameRgba, OcrEngine},
    types::LanguageTag,
};

/// Result from the combined OCR + Translation call.
pub struct OcrTranslateResult {
    pub ocr_text: String,
    pub translated: String,
}

/// OCR adapter that uses Gemini Vision to extract text from screenshots.
/// Also supports a combined OCR+Translate call that halves API usage.
#[derive(Clone)]
pub struct GeminiOcr {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiOcr {
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("build http client")?;
        Ok(Self {
            client,
            api_key,
            model,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        )
    }

    /// Encode a FrameRgba to base64-encoded PNG.
    fn encode_frame_base64(frame: &FrameRgba) -> Result<String> {
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(frame.width, frame.height, frame.data.clone())
                .context("invalid frame dimensions")?;

        let dynamic = image::DynamicImage::ImageRgba8(img);
        let mut png_bytes: Vec<u8> = Vec::new();
        dynamic
            .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .context("encode frame to PNG")?;

        Ok(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
    }

    /// Combined OCR + Translation in a single Gemini Vision call.
    /// This halves API usage compared to separate OCR and Translate calls.
    pub fn recognize_and_translate(
        &self,
        frame: &FrameRgba,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<OcrTranslateResult> {
        if self.api_key.trim().is_empty() {
            bail!("Gemini API key is empty (open Settings and set it)");
        }

        let b64 = Self::encode_frame_base64(frame)?;

        let prompt = if let Some(src) = source {
            format!(
                "You see a screenshot. Do two things:\n\
                 1. Extract ALL readable text from the image exactly as written.\n\
                 2. Translate that text from {} to {}.\n\
                 Respond with JSON: {{\"ocr\": \"<the extracted text>\", \"translation\": \"<the translation>\"}}\n\
                 If no readable text, respond: {{\"ocr\": \"\", \"translation\": \"\"}}",
                src.0, target.0
            )
        } else {
            format!(
                "You see a screenshot. Do two things:\n\
                 1. Extract ALL readable text from the image exactly as written.\n\
                 2. Auto-detect the source language and translate to {}.\n\
                 Respond with JSON: {{\"ocr\": \"<the extracted text>\", \"translation\": \"<the translation>\"}}\n\
                 If no readable text, respond: {{\"ocr\": \"\", \"translation\": \"\"}}",
                target.0
            )
        };

        let req_body = serde_json::json!({
            "contents": [{
                "parts": [
                    { "text": prompt },
                    { "inline_data": { "mime_type": "image/png", "data": b64 } }
                ]
            }],
            "generationConfig": {
                "temperature": 0.1,
                "maxOutputTokens": 4096,
                "responseMimeType": "application/json"
            }
        });

        let resp = self
            .client
            .post(self.endpoint())
            .header("x-goog-api-key", &self.api_key)
            .json(&req_body)
            .send()
            .context("send Gemini request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Gemini error: {status} {body}");
        }

        let data: serde_json::Value = resp.json().context("parse response")?;
        let raw = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim();

        // With responseMimeType the reply should be valid JSON directly.
        // Fallback: treat it as raw OCR text if JSON parsing fails.
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap_or_else(|_| {
            serde_json::json!({"ocr": raw, "translation": ""})
        });

        Ok(OcrTranslateResult {
            ocr_text: parsed["ocr"].as_str().unwrap_or("").trim().to_string(),
            translated: parsed["translation"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string(),
        })
    }
}

impl OcrEngine for GeminiOcr {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String> {
        if self.api_key.trim().is_empty() {
            bail!("Gemini API key is empty (open Settings and set it)");
        }

        let b64 = Self::encode_frame_base64(frame)?;

        let prompt = if let Some(lang) = lang_hint {
            format!(
                "Extract all text from this image. The text is in {}. \
                 Output ONLY the raw text content, nothing else. \
                 If there is no readable text, output nothing.",
                lang.0
            )
        } else {
            "Extract all text from this image. \
             Output ONLY the raw text content, nothing else. \
             If there is no readable text, output nothing."
                .to_string()
        };

        let req_body = serde_json::json!({
            "contents": [{
                "parts": [
                    { "text": prompt },
                    { "inline_data": { "mime_type": "image/png", "data": b64 } }
                ]
            }],
            "generationConfig": {
                "temperature": 0.1,
                "maxOutputTokens": 4096
            }
        });

        let resp = self
            .client
            .post(self.endpoint())
            .header("x-goog-api-key", &self.api_key)
            .json(&req_body)
            .send()
            .context("send Gemini OCR request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Gemini OCR error: {status} {body}");
        }

        let data: serde_json::Value = resp.json().context("parse Gemini OCR response")?;
        let text = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(text)
    }
}
