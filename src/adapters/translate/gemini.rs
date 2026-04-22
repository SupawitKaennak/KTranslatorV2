use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::core::{ports::Translator, types::LanguageTag};

#[derive(Debug, Clone)]
pub struct GeminiModel {
    pub id: String,          // "gemini-2.5-flash"
    pub display_name: String, // "Gemini 2.5 Flash"
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
            // name is typically like "models/gemini-2.5-flash"
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
        let prompt = if let Some(src) = source {
            format!(
                "Translate from {} to {}. Output ONLY the translation text.\n\n{}",
                src.0, target.0, text
            )
        } else {
            format!(
                "Translate to {}. Auto-detect the source language. Output ONLY the translation text.\n\n{}",
                target.0, text
            )
        };

        let req = GenerateContentRequest {
            contents: vec![Content {
                parts: vec![Part { text: prompt }],
            }],
            generation_config: Some(GenerationConfig {
                temperature: Some(0.2),
                top_p: Some(0.95),
                max_output_tokens: Some(512),
                // Disable reasoning tokens — translation is deterministic.
                // Ignored by models that don't support it; saves tokens on 2.5-flash.
                thinking_config: Some(ThinkingConfig { thinking_budget: 0 }),
            }),
        };

        let resp = self
            .client
            .post(self.endpoint())
            .header("x-goog-api-key", self.api_key.clone())
            .json(&req)
            .send()
            .context("send gemini request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("Gemini error: {status} {body}");
        }

        let data: GenerateContentResponse = resp.json().context("parse gemini response")?;
        let out = data
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content)
            .and_then(|c| c.parts.into_iter().next())
            .map(|p| p.text)
            .unwrap_or_default();

        Ok(out.trim().to_string())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
struct Part {
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    /// Disable thinking tokens for 2.5-flash models (translation is a
    /// simple deterministic task — thinking adds cost with no benefit).
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<ThinkingConfig>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingConfig {
    thinking_budget: u32,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<CandidateContent>,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<CandidatePart>,
}

#[derive(Debug, Deserialize)]
struct CandidatePart {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct ListModelsResponse {
    #[serde(default)]
    models: Vec<ModelInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfo {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    supported_generation_methods: Option<Vec<String>>,
}

