use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::Mutex;
use crate::infra::settings::{Settings, TranslationProvider};
use crate::adapters::translate::gemini::GeminiModel;

pub struct SettingsWindowResponse {
    pub save_clicked: bool,
    pub close_clicked: bool,
}

#[derive(serde::Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

#[derive(serde::Deserialize)]
struct OpenAiModelItem {
    id: String,
}

/// Renders the settings viewport. 
/// Returns a response indicating if save or close was requested.
pub fn show_settings_window(
    ctx: &egui::Context,
    settings_arc: Arc<Mutex<Settings>>,
    model_choices: Vec<GeminiModel>,
    custom_models: Arc<Mutex<Vec<String>>>,
    custom_fetching: Arc<Mutex<bool>>,
    custom_error: Arc<Mutex<Option<String>>>,
) -> SettingsWindowResponse {
    let save_flag = Arc::new(AtomicBool::new(false));
    let close_flag = Arc::new(AtomicBool::new(false));
    
    let save_flag_inner = save_flag.clone();
    let close_flag_inner = close_flag.clone();
    let settings_inner = settings_arc.clone();
    
    let viewport_id = egui::ViewportId::from_hash_of("settings_viewport");

    ctx.show_viewport_immediate(
        viewport_id,
        egui::ViewportBuilder::default()
            .with_title("KTranslator - Settings")
            .with_inner_size([550.0, 600.0])
            .with_resizable(true)
            .with_always_on_top(),
        move |ctx, _| {
            if ctx.input(|i| i.viewport().close_requested()) {
                close_flag_inner.store(true, Ordering::Relaxed);
            }

            egui::CentralPanel::default().show(ctx, |ui| {
                let mut settings = settings_inner.lock();

                ui.heading("⚙ Settings");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Provider:");
                    ui.selectable_value(
                        &mut settings.provider,
                        TranslationProvider::Gemini,
                        "Gemini",
                    );
                    ui.selectable_value(
                        &mut settings.provider,
                        TranslationProvider::Groq,
                        "Groq",
                    );
                    ui.selectable_value(
                        &mut settings.provider,
                        TranslationProvider::Ollama,
                        "Ollama (Offline)",
                    );
                    ui.selectable_value(
                        &mut settings.provider,
                        TranslationProvider::CustomOpenAI,
                        "Custom (OpenAI Compatible)",
                    );
                });
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("OCR Engine:");
                    ui.selectable_value(
                        &mut settings.ocr_engine,
                        crate::infra::settings::OcrEngineType::Windows,
                        "Windows OCR (Default)",
                    );
                    ui.selectable_value(
                        &mut settings.ocr_engine,
                        crate::infra::settings::OcrEngineType::Paddle,
                        "PaddleOCR (Best for Manga)",
                    );
                });
                
                if settings.ocr_engine == crate::infra::settings::OcrEngineType::Paddle {
                    ui.horizontal(|ui| {
                        ui.label("PaddleOCR-json path:");
                        ui.add(egui::TextEdit::singleline(&mut settings.paddle_ocr_path).hint_text("C:\\path\\to\\PaddleOCR-json.exe"));
                    });
                    ui.small("Download PaddleOCR-json for the best manga recognition results.");
                }

                ui.separator();

                match settings.provider {
                    TranslationProvider::Gemini => {
                        ui.label("Gemini (API key)");
                        ui.horizontal(|ui| {
                            ui.label("model");
                            let mut current_idx = model_choices
                                .iter()
                                .position(|m| m.id == settings.gemini_model)
                                .unwrap_or(0);
                            egui::ComboBox::from_id_salt("gemini_model_dropdown")
                                .width(250.0)
                                .selected_text(
                                    model_choices
                                        .get(current_idx)
                                        .map(|m| m.display_name.as_str())
                                        .unwrap_or(settings.gemini_model.as_str()),
                                )
                                .show_ui(ui, |ui| {
                                    for (i, m) in model_choices.iter().enumerate() {
                                        ui.selectable_value(&mut current_idx, i, &m.display_name);
                                    }
                                });
                            if let Some(sel) = model_choices.get(current_idx) {
                                settings.gemini_model = sel.id.clone();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("api_key");
                            ui.add(
                                egui::TextEdit::singleline(&mut settings.gemini_api_key)
                                    .password(true),
                            );
                        });
                    }
                    TranslationProvider::Groq => {
                        ui.label("Groq (High speed, Free)");
                        ui.horizontal(|ui| {
                            ui.label("model");
                            let groq_models = vec![
                                ("llama-3.3-70b-versatile", "Llama 3.3 70B (Versatile)"),
                                ("llama-3.1-8b-instant", "Llama 3.1 8B (Instant)"),
                                ("gemma2-9b-it", "Gemma 2 9B"),
                                ("qwen-qwq-32b", "Qwen QwQ 32B (Reasoning)"),
                                ("mistral-saba-24b", "Mistral Saba 24B"),
                                ("deepseek-r1-distill-llama-70b", "DeepSeek R1 70B"),
                            ];
                            let mut current = settings.groq_model.clone();
                            egui::ComboBox::from_id_salt("groq_model_dropdown")
                                .width(280.0)
                                .selected_text(
                                    groq_models.iter()
                                        .find(|(id, _)| *id == current.as_str())
                                        .map(|(_, name)| *name)
                                        .unwrap_or_else(|| current.as_str()),
                                )
                                .show_ui(ui, |ui| {
                                    for (id, name) in &groq_models {
                                        ui.selectable_value(&mut current, id.to_string(), *name);
                                    }
                                });
                            settings.groq_model = current;
                        });
                        ui.horizontal(|ui| {
                            ui.label("api_key");
                            ui.add(
                                egui::TextEdit::singleline(&mut settings.groq_api_key)
                                    .password(true),
                            );
                        });
                        ui.hyperlink_to("Get Groq API Key", "https://console.groq.com/keys");
                    }
                    TranslationProvider::Ollama => {
                        ui.label("Ollama (Local/Offline — Unlimited & Free)");
                        ui.horizontal(|ui| {
                            ui.label("Server URL");
                            ui.text_edit_singleline(&mut settings.ollama_url);
                        });
                        ui.add_space(4.0);

                        let ollama_models: Vec<(&str, &str, &str)> = vec![
                            ("qwen2.5:0.5b",     "Qwen 2.5 0.5B",     "⚡ Ultra-light, CPU OK (~1GB)"),
                            ("qwen2.5:1.5b",     "Qwen 2.5 1.5B",     "⚡ Very light, CPU OK (~2GB)"),
                            ("qwen2.5:3b",       "Qwen 2.5 3B",       "⚡ Light & capable (~3GB)"),
                            ("llama3.2:1b",      "Llama 3.2 1B",      "⚡ Meta ultra-light (~2GB)"),
                            ("llama3.2:3b",      "Llama 3.2 3B",      "⚡ Meta light (~3GB)"),
                            ("gemma2:2b",        "Gemma 2 2B",        "⚡ Google ultra-light (~2GB)"),
                            ("phi3:mini",        "Phi-3 Mini 3.8B",   "⚡ Microsoft light (~3GB)"),
                            ("qwen2.5:7b",       "Qwen 2.5 7B",       "🌟 Best for Asian languages (8GB)"),
                            ("qwen3:8b",         "Qwen 3 8B",         "🆕 Latest Qwen (8GB)"),
                            ("gemma2:9b",        "Gemma 2 9B",        "Google efficient (8GB)"),
                            ("aya-expanse:8b",   "Aya Expanse 8B",    "🌐 Multilingual specialist (8GB)"),
                            ("llama3.1:8b",      "Llama 3.1 8B",      "Meta versatile (8GB)"),
                            ("qwen2.5:14b",      "Qwen 2.5 14B",      "🌟 High quality Asian (12GB)"),
                            ("qwen3:14b",        "Qwen 3 14B",        "🆕 Latest Qwen (12GB)"),
                            ("gemma2:27b",       "Gemma 2 27B",       "Google premium (20GB)"),
                            ("qwen2.5:32b",      "Qwen 2.5 32B",      "🏆 Premium quality (24GB)"),
                            ("qwen3:32b",        "Qwen 3 32B",        "🆕 Latest Qwen (24GB)"),
                            ("aya-expanse:32b",  "Aya Expanse 32B",   "🌐 Best multilingual (24GB)"),
                            ("qwen2.5:72b",      "Qwen 2.5 72B",      "🏆 Near GPT-4 (48GB+)"),
                            ("llama3.3:70b",     "Llama 3.3 70B",     "Meta flagship (48GB+)"),
                        ];

                        ui.horizontal(|ui| {
                            ui.label("Model");
                            let mut current = settings.ollama_model.clone();
                            egui::ComboBox::from_id_salt("ollama_model_dropdown")
                                .width(300.0)
                                .selected_text(
                                    ollama_models.iter()
                                        .find(|(id, _, _)| *id == current.as_str())
                                        .map(|(_, name, _)| *name)
                                        .unwrap_or_else(|| current.as_str()),
                                )
                                .show_ui(ui, |ui| {
                                    for (id, name, desc) in &ollama_models {
                                        let label = format!("{name}  —  {desc}");
                                        ui.selectable_value(&mut current, id.to_string(), label);
                                    }
                                });
                            settings.ollama_model = current;
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Custom");
                            ui.text_edit_singleline(&mut settings.ollama_model)
                                .on_hover_text("Type any model name here if it's not in the dropdown");
                        });

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);

                        let pull_cmd = format!("ollama pull {}", settings.ollama_model);
                        ui.horizontal(|ui| {
                            ui.label("📋 Run this first:");
                            if ui.button(&pull_cmd).on_hover_text("Click to copy").clicked() {
                                ui.ctx().copy_text(pull_cmd.clone());
                            }
                        });
                        ui.small("Make sure Ollama is running before clicking Save & Apply.");
                    }
                    TranslationProvider::CustomOpenAI => {
                        ui.label("Custom API (OpenAI Compatible) — LM Studio, OpenRouter, DeepSeek, etc.");
                        ui.horizontal(|ui| {
                            ui.label("Base URL");
                            ui.text_edit_singleline(&mut settings.custom_openai_url)
                                .on_hover_text("e.g. http://localhost:1234/v1 or https://openrouter.ai/api/v1");
                        });
                        
                        ui.horizontal(|ui| {
                            ui.label("Model Name");
                            
                            let current_model = settings.custom_openai_model.clone();
                            let models = custom_models.lock();
                            let is_fetching = *custom_fetching.lock();

                            if !models.is_empty() {
                                let mut selected = current_model.clone();
                                egui::ComboBox::from_id_salt("custom_openai_model_dropdown")
                                    .width(200.0)
                                    .selected_text(&selected)
                                    .show_ui(ui, |ui| {
                                        for m in models.iter() {
                                            ui.selectable_value(&mut selected, m.clone(), m);
                                        }
                                    });
                                settings.custom_openai_model = selected;
                            }

                            ui.add(egui::TextEdit::singleline(&mut settings.custom_openai_model)
                                .hint_text("Manual entry or select from list"));

                            if is_fetching {
                                ui.spinner();
                            } else {
                                if ui.button("🔄 Fetch").on_hover_text("Try to fetch model list from API").clicked() {
                                    let url = settings.custom_openai_url.clone();
                                    let key = settings.custom_openai_api_key.clone();
                                    let models_arc = custom_models.clone();
                                    let fetching_arc = custom_fetching.clone();
                                    let error_arc = custom_error.clone();
                                    let ctx_clone = ctx.clone();

                                    *custom_fetching.lock() = true;
                                    *custom_error.lock() = None;

                                    std::thread::spawn(move || {
                                        let client = reqwest::blocking::Client::builder()
                                            .timeout(std::time::Duration::from_secs(10))
                                            .build();
                                        
                                        match client {
                                            Ok(c) => {
                                                let endpoint = format!("{}/models", url.trim_end_matches('/'));
                                                let mut req = c.get(&endpoint);
                                                if !key.trim().is_empty() {
                                                    req = req.bearer_auth(key.trim());
                                                }
                                                
                                                match req.send() {
                                                    Ok(resp) => {
                                                        if resp.status().is_success() {
                                                            let text = resp.text().unwrap_or_default();
                                                            match serde_json::from_str::<OpenAiModelsResponse>(&text) {
                                                                Ok(parsed) => {
                                                                    let mut m_list = parsed.data.into_iter().map(|i| i.id).collect::<Vec<_>>();
                                                                    m_list.sort();
                                                                    *models_arc.lock() = m_list;
                                                                }
                                                                Err(e) => {
                                                                    *error_arc.lock() = Some(format!("Parse error: {e}"));
                                                                }
                                                            }
                                                        } else {
                                                            *error_arc.lock() = Some(format!("API error: {}", resp.status()));
                                                        }
                                                    }
                                                    Err(e) => {
                                                        *error_arc.lock() = Some(format!("Network error: {e}"));
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                *error_arc.lock() = Some(format!("Client error: {e}"));
                                            }
                                        }
                                        *fetching_arc.lock() = false;
                                        ctx_clone.request_repaint();
                                    });
                                }
                            }
                        });

                        if let Some(err) = custom_error.lock().as_ref() {
                            ui.colored_label(egui::Color32::RED, format!("⚠ {err}"));
                        }

                        ui.horizontal(|ui| {
                            ui.label("API Key (Optional)");
                            ui.add(egui::TextEdit::singleline(&mut settings.custom_openai_api_key).password(true))
                                .on_hover_text("Leave blank if connecting to local LM Studio or Ollama");
                        });
                        ui.small("The URL should point to the base path, /chat/completions will be appended automatically.");
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                ui.heading("📺 Overlay Appearance");
                egui::Grid::new("overlay_settings_grid")
                    .num_columns(2)
                    .spacing([20.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Background Color:");
                        let mut bg_color = egui::Color32::from_rgba_unmultiplied(
                            settings.overlay_bg_color[0],
                            settings.overlay_bg_color[1],
                            settings.overlay_bg_color[2],
                            settings.overlay_bg_color[3],
                        );
                        if ui.color_edit_button_srgba(&mut bg_color).changed() {
                            settings.overlay_bg_color = bg_color.to_array();
                        }
                        ui.end_row();

                        ui.label("Text Color:");
                        let mut text_color = egui::Color32::from_rgba_unmultiplied(
                            settings.overlay_text_color[0],
                            settings.overlay_text_color[1],
                            settings.overlay_text_color[2],
                            settings.overlay_text_color[3],
                        );
                        if ui.color_edit_button_srgba(&mut text_color).changed() {
                            settings.overlay_text_color = text_color.to_array();
                        }
                        ui.end_row();

                        ui.label("Font Size:");
                        ui.add(egui::Slider::new(&mut settings.overlay_font_size, 8.0..=48.0).suffix("px"));
                        ui.end_row();

                        ui.label("Padding:");
                        ui.add(egui::Slider::new(&mut settings.overlay_padding, 0.0..=20.0).suffix("px"));
                        ui.end_row();

                        ui.label("Corner Radius:");
                        ui.add(egui::Slider::new(&mut settings.overlay_corner_radius, 0.0..=20.0).suffix("px"));
                        ui.end_row();
                    });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);

                if ui.button(egui::RichText::new("💾 Save & Apply").size(16.0)).clicked() {
                    save_flag_inner.store(true, Ordering::Relaxed);
                }
            });
        },
    );
    
    SettingsWindowResponse {
        save_clicked: save_flag.load(Ordering::Relaxed),
        close_clicked: close_flag.load(Ordering::Relaxed),
    }
}
