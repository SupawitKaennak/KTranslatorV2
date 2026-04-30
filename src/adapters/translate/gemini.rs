use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::core::{ports::Translator, types::LanguageTag};

#[derive(Debug, Clone)]
pub struct GeminiModel {
    pub id: String,          // "gemini-2.0-flash"
    pub display_name: String, // "Gemini 2.0 Flash"
}

#[derive(Clone)]
pub struct GeminiTranslator {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiTranslator {
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("build http client")?;
        Ok(Self {
            client,
            api_key,
            model,
        })
    }

    pub fn list_models(api_key: &str) -> Result<Vec<GeminiModel>> {
        if api_key.trim().is_empty() {
            bail!("Gemini API key is empty");
        }
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("build http client")?;

        let resp = client
            .get("https://generativelanguage.googleapis.com/v1beta/models")
            .query(&[("key", api_key)])
            .send()
            .context("send listModels request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Gemini listModels error: {status} {body}");
        }

        let data: ListModelsResponse = resp.json().context("parse listModels response")?;
        let mut out = Vec::new();
        for m in data.models {
            let id = m
                .name
                .strip_prefix("models/")
                .unwrap_or(m.name.as_str())
                .to_string();
            let display_name = m.display_name.unwrap_or_else(|| id.clone());
            if m.supported_generation_methods
                .as_ref()
                .map(|xs| xs.iter().any(|x| x == "generateContent"))
                .unwrap_or(true)
            {
                out.push(GeminiModel { id, display_name });
            }
        }
        out.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
        Ok(out)
    }

    fn endpoint(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        )
    }
}

impl Translator for GeminiTranslator {
    fn translate(
        &self,
        text: &str,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<String> {
        if self.api_key.trim().is_empty() {
            bail!("Gemini API key is empty (open Settings and set it)");
        }

        let source_lines: Vec<&str> = text.lines().collect();
        let (prompt_body, multi_line) = if source_lines.len() > 1 {
            let numbered = source_lines
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{}. {}", i + 1, l))
                .collect::<Vec<_>>()
                .join("\n");
            (numbered, true)
        } else {
            (text.to_string(), false)
        };

        let line_instruction = if multi_line {
            "CRITICAL: The input is a numbered list of lines. \
             You MUST return EXACTLY the same number of lines as provided. \
             Each output line must start with its corresponding number (N. <translation>). \
             Do NOT skip, merge, or omit any lines. \
             Even if a line is short or empty, you must include its number. \
             Return ONLY the numbered list, no intro, no outro, no notes."
                .to_string()
        } else {
            "Output ONLY the translated text, no explanations.".to_string()
        };

        let system_instruction = "You are a professional game localizer. \
             Translate the following text to sound natural, idiomatic, and human-like in the target language. \
             Avoid literal, word-for-word translations that sound like a robot. \
             Use appropriate gaming terminology and casual speech where suitable.";

        let prompt = if let Some(src) = source {
            format!(
                "{}\n\nTranslate from {} to {}. {}\n\nInput:\n{}",
                system_instruction, src.0, target.0, line_instruction, prompt_body
            )
        } else {
            format!(
                "{}\n\nTranslate to {}. Auto-detect the source language. {}\n\nInput:\n{}",
                system_instruction, target.0, line_instruction, prompt_body
            )
        };

        let body = RequestBody {
            contents: vec![Content {
                parts: vec![Part { text: prompt }],
            }],
            generation_config: Some(GenerationConfig {
                temperature: Some(0.3), // Slightly higher for more natural phrasing
                max_output_tokens: Some(4096),
                ..Default::default()
            }),
        };

        let resp = self.client
            .post(self.endpoint())
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .context("send generateContent request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Gemini API error: {status} {body}");
        }

        let data: ResponseBody = resp.json().context("parse generateContent response")?;
        let translated = data
            .candidates
            .get(0)
            .and_then(|c| c.content.parts.get(0))
            .map(|p| p.text.clone())
            .ok_or_else(|| anyhow::anyhow!("Gemini returned no candidates (Safety filter?)"))?;

        Ok(translated)
    }
}

#[derive(Serialize)]
struct RequestBody {
    contents: Vec<Content>,
    #[serde(rename = "generationConfig")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize, Deserialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Serialize, Deserialize)]
struct Part {
    text: String,
}

#[derive(Serialize, Default)]
struct GenerationConfig {
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ResponseBody {
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Deserialize)]
struct ListModelsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    name: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "supportedGenerationMethods")]
    supported_generation_methods: Option<Vec<String>>,
}
