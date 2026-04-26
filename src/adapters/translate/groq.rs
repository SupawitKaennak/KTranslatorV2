use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::core::{ports::Translator, types::LanguageTag};

#[derive(Clone)]
pub struct GroqTranslator {
    client: Client,
    api_key: String,
    model: String,
}

impl GroqTranslator {
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
}

impl Translator for GroqTranslator {
    fn translate(
        &self,
        text: &str,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<String> {
        if self.api_key.trim().is_empty() {
            bail!("Groq API key is empty (obtain it from console.groq.com)");
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

        let system_prompt = if multi_line {
            format!(
                "You are a professional translator. Translate each numbered line into {}. \
                 Keep the same numbers. Output ONLY the translated numbered list.",
                target_name
            )
        } else {
            format!(
                "You are a professional translator. Translate this text into {}. \
                 Output ONLY the translated text.",
                target_name
            )
        };

        let user_prompt = if let Some(src) = source {
            let src_name = match src.0.as_str() {
                "th" => "Thai",
                "en" => "English",
                "ja" => "Japanese",
                "zh-Hans" => "Chinese Simplified",
                "zh-Hant" => "Chinese Traditional",
                "ko" => "Korean",
                _ => &src.0,
            };
            format!("Translate from {} to {}:\n\n{}", src_name, target_name, prompt_body)
        } else {
            format!("Translate to {}:\n\n{}", target_name, prompt_body)
        };

        let req = GroqChatRequest {
            model: self.model.clone(),
            messages: vec![
                GroqMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                GroqMessage {
                    role: "user".to_string(),
                    content: user_prompt.to_string(),
                },
            ],
            temperature: 0.2,
            max_tokens: 4096,
        };

        let resp = self.client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&req)
            .send()
            .context("send groq request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Groq error: {status} {body}");
        }

        let data: GroqChatResponse = resp.json().context("parse groq response")?;
        let out = data.choices.into_iter().next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        Ok(out.trim().to_string())
    }
}

#[derive(Serialize)]
struct GroqChatRequest {
    model: String,
    messages: Vec<GroqMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct GroqMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct GroqChatResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Deserialize)]
struct GroqChoice {
    message: GroqMessageResponse,
}

#[derive(Deserialize)]
struct GroqMessageResponse {
    content: String,
}
