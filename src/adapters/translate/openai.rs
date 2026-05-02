use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::core::{ports::Translator, types::LanguageTag};

#[derive(Clone)]
pub struct OpenAiTranslator {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiTranslator {
    pub fn new(base_url: String, api_key: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .pool_idle_timeout(std::time::Duration::from_secs(120))
            .build()
            .context("build http client")?;
            
        let base_url = base_url.trim_end_matches('/').to_string();
        
        Ok(Self {
            client,
            base_url,
            api_key,
            model,
        })
    }
}

impl Translator for OpenAiTranslator {
    fn translate(
        &self,
        text: &str,
        source: Option<&LanguageTag>,
        target: &LanguageTag,
    ) -> Result<String> {
        if self.base_url.is_empty() {
            bail!("Custom OpenAI Base URL is empty");
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

        let system_prompt = if multi_line {
            format!(
                "You are an expert manga/game translator. Translate each numbered line to {target_lang}. \
                Source language: {source_lang}. \
                Maintain context across lines as they belong to the same scene or speech bubble. \
                RULES:
                1. You MUST return EXACTLY the same number of lines.
                2. Each output line MUST start with its corresponding number (e.g., '1. <translation>').
                3. Do not add extra commentary, notes, or combine lines.
                4. Maintain punctuation style appropriate for {target_lang}.",
                target_lang = target.0,
                source_lang = source.map(|l| l.0.as_str()).unwrap_or("auto-detect"),
            )
        } else {
            format!(
                "You are an expert manga/game translator. Translate the text to {target_lang}. \
                Source language: {source_lang}. \
                RULES:
                1. Provide ONLY the translation.
                2. Do not add any commentary, notes, or quotes.",
                target_lang = target.0,
                source_lang = source.map(|l| l.0.as_str()).unwrap_or("auto-detect"),
            )
        };

        let req_body = OpenAiRequest {
            model: self.model.clone(),
            messages: vec![
                OpenAiMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                OpenAiMessage {
                    role: "user".to_string(),
                    content: prompt_body,
                },
            ],
            temperature: 0.3,
        };

        let endpoint = format!("{}/chat/completions", self.base_url);
        
        let mut req = self.client.post(&endpoint);
        if !self.api_key.trim().is_empty() {
            req = req.bearer_auth(self.api_key.trim());
        }

        let res = req
            .json(&req_body)
            .send()
            .context("OpenAI compatible request failed")?;

        let status = res.status();
        let body_text = res.text().unwrap_or_default();

        if !status.is_success() {
            bail!("OpenAI API error {}: {}", status, body_text);
        }

        let resp: OpenAiResponse = serde_json::from_str(&body_text)
            .with_context(|| format!("Failed to parse OpenAI API response: {}", body_text))?;

        let translated = resp
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .map(|m| m.content.trim().to_string())
            .unwrap_or_default();

        Ok(translated)
    }
}

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: Option<OpenAiMessage>,
}
