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

        let system = if source_lines.len() > 1 {
            format!(
                "Translate each line into {}. \
                 Return EXACTLY {} lines of translation. \
                 Each input line must have exactly one output line. \
                 Do NOT add numbers, bullets, or explanations. \
                 Do NOT merge or skip lines. \
                 Keep blank lines as blank lines. \
                 Output ONLY the translated lines, one per line.",
                target_name, source_lines.len()
            )
        } else {
            format!(
                "Translate into {}. Output ONLY the translation, nothing else.",
                target_name
            )
        };

        let user = if let Some(sn) = src_name {
            format!("Translate from {} to {}:\n\n{}", sn, target_name, text)
        } else {
            format!("Translate to {}:\n\n{}", target_name, text)
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
        Ok(data.message.content.trim().to_string())
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
