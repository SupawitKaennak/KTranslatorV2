use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::core::{ports::Translator, types::LanguageTag};

#[derive(Clone)]
pub struct OllamaTranslator {
    client: Client,
    url: String, // e.g. "http://localhost:11434"
    model: String,
}

impl OllamaTranslator {
    pub fn new(url: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // Local models can be very slow on CPU
            .build()
            .context("build http client")?;
        Ok(Self {
            client,
            url: url.trim_end_matches('/').to_string(),
            model,
        })
    }
}

impl Translator for OllamaTranslator {
    fn translate(
        &self,
        text: &str,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<String> {
        let source_lines: Vec<&str> = text.lines().collect();

        // Convert language codes to full names for better AI understanding
        let target_name = match target.0.as_str() {
            "th" => "Thai",
            "en" => "English",
            "ja" => "Japanese",
            "zh-Hans" => "Chinese Simplified",
            "zh-Hant" => "Chinese Traditional",
            "ko" => "Korean",
            "vi" => "Vietnamese",
            "id" => "Indonesian",
            "ru" => "Russian",
            "es" => "Spanish",
            "fr" => "French",
            "de" => "German",
            _ => &target.0,
        };

        let src_name = source.map(|s| match s.0.as_str() {
            "th" => "Thai",
            "en" => "English",
            "ja" => "Japanese",
            "zh-Hans" => "Chinese Simplified",
            "zh-Hant" => "Chinese Traditional",
            "ko" => "Korean",
            _ => &s.0,
        });

        // For multi-line: translate each line individually to guarantee alignment.
        // Local models are unlimited, so multiple calls are fine.
        if source_lines.len() > 1 {
            let mut results = Vec::with_capacity(source_lines.len());
            for (i, line) in source_lines.iter().enumerate() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    results.push(String::new());
                    continue;
                }

                // Include surrounding lines as context (but only translate the target line)
                let context_hint = if source_lines.len() > 1 {
                    let prev = if i > 0 { source_lines[i - 1] } else { "" };
                    let next = if i + 1 < source_lines.len() { source_lines[i + 1] } else { "" };
                    format!(
                        " Context — previous line: \"{}\", next line: \"{}\".",
                        prev, next
                    )
                } else {
                    String::new()
                };

                let system = format!(
                    "Translate into {}. Output ONLY the translation, nothing else. No numbers, no bullet points, no explanations.{}",
                    target_name, context_hint
                );

                let user = if let Some(sn) = src_name {
                    format!("{} → {}: {}", sn, target_name, trimmed)
                } else {
                    trimmed.to_string()
                };

                let translated_line = self.call_ollama(&system, &user)?;
                results.push(translated_line);
            }
            return Ok(results.join("\n"));
        }

        // Single line — simple translation
        let system = format!(
            "Translate into {}. Output ONLY the translation, nothing else.",
            target_name
        );
        let user = if let Some(sn) = src_name {
            format!("{} → {}: {}", sn, target_name, text.trim())
        } else {
            text.trim().to_string()
        };

        self.call_ollama(&system, &user)
    }
}

impl OllamaTranslator {
    fn call_ollama(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let req = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: user_prompt.to_string(),
                },
            ],
            stream: false,
            options: Some(OllamaOptions {
                temperature: 0.2,
                num_predict: 4096,
            }),
        };

        let endpoint = format!("{}/api/chat", self.url);
        let resp = self.client
            .post(&endpoint)
            .json(&req)
            .send()
            .context("send ollama request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Ollama error: {status} {body} (Make sure Ollama is running and model '{}' is pulled)", self.model);
        }

        let data: OllamaChatResponse = resp.json().context("parse ollama response")?;
        // Take only the first non-empty line to prevent multi-line leakage
        let content = data.message.content.trim();
        let first_line = content.lines().next().unwrap_or(content).trim();
        Ok(first_line.to_string())
    }
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
    num_predict: u32,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}
