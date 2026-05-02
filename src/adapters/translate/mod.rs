pub mod gemini;
pub mod groq;
pub mod ollama;
pub mod openai;

use std::sync::Arc;
use crate::core::ports::Translator;
use crate::infra::settings::{Settings, TranslationProvider};
use self::gemini::GeminiTranslator;
use self::groq::GroqTranslator;
use self::ollama::OllamaTranslator;
use self::openai::OpenAiTranslator;

pub fn create_translator(settings: &Settings) -> Option<Arc<dyn Translator + Send + Sync>> {
    match settings.provider {
        TranslationProvider::Gemini => GeminiTranslator::new(
            settings.gemini_api_key.clone(),
            settings.gemini_model.clone(),
        )
        .ok()
        .map(|t| Arc::new(t) as Arc<dyn Translator + Send + Sync>),
        TranslationProvider::Groq => {
            GroqTranslator::new(settings.groq_api_key.clone(), settings.groq_model.clone())
                .ok()
                .map(|t| Arc::new(t) as Arc<dyn Translator + Send + Sync>)
        }
        TranslationProvider::Ollama => OllamaTranslator::new(
            settings.ollama_url.clone(),
            settings.ollama_model.clone(),
        )
        .ok()
        .map(|t| Arc::new(t) as Arc<dyn Translator + Send + Sync>),
        TranslationProvider::CustomOpenAI => OpenAiTranslator::new(
            settings.custom_openai_url.clone(),
            settings.custom_openai_api_key.clone(),
            settings.custom_openai_model.clone(),
        )
        .ok()
        .map(|t| Arc::new(t) as Arc<dyn Translator + Send + Sync>),
    }
}
