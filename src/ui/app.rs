use std::sync::{mpsc, Arc};

use eframe::egui;
use parking_lot::Mutex;

use crate::{
    adapters::{
        capture::screenshots_capture::ScreenshotsCapture,
        ocr::windows_ocr::WindowsOcr,
        translate::{
            gemini::{GeminiModel, GeminiTranslator},
            groq::GroqTranslator,
            ollama::OllamaTranslator,
        },
    },
    core::{
        model::AppModel,
        ports::{FrameRgba, FrameSource, OcrTextLine, Translator},
        types::{LanguageTag, Rect},
    },
    infra::settings::{load_settings, save_settings, Settings, TranslationProvider},
    ui::crop_overlay::{run_crop_viewport, CropOutcome, CropOverlayState},
};

// ---------------------------------------------------------------------------
// Background-thread result messages
// ---------------------------------------------------------------------------

enum BgResult {
    /// Combined OCR + Translation completed successfully.
    Done {
        slot_idx: usize,
        language_version: u32,
        ocr_text: String,
        translated: String,
        frame_hash: u64,
        /// Per-line OCR bounding boxes for positional overlay rendering.
        ocr_lines: Vec<OcrTextLine>,
    },
    /// The captured frame is identical to the previous one — skip API call.
    Unchanged {
        slot_idx: usize,
    },
    /// The screen is changing. Update the stable hash tracker and skip API.
    HashChanged {
        slot_idx: usize,
        new_hash: u64,
    },
    /// The screen is stable but we are waiting for the debounce duration.
    WaitingDebounce {
        slot_idx: usize,
    },
    /// The frame matches a cached translation.
    CacheHit {
        slot_idx: usize,
        language_version: u32,
        ocr_text: String,
        translated: String,
        frame_hash: u64,
    },
    /// Background thread is now engaging Gemini or OCR (heavy work)
    Translating {
        slot_idx: usize,
    },
    StatusUpdate {
        slot_idx: usize,
        status: String,
    },
    /// An error occurred during OCR / Translation.
    Error {
        slot_idx: usize,
        language_version: u32,
        err: String,
    },
}

// ---------------------------------------------------------------------------
// Simple fast hash of raw pixel data to detect screen changes
// ---------------------------------------------------------------------------

/// Smart hash converts RGBA to thresholded grayscale before hashing.
/// This prevents minor lighting/background particle changes from triggering text translation.
fn smart_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    
    // Sample every 4th pixel (16 bytes) for a balance of speed and precision
    let step: usize = 16;
    let mut i = 0;
    while i + 2 < data.len() {
        let r = data[i] as f32;
        let g = data[i+1] as f32;
        let b = data[i+2] as f32;
        
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        let bw = if lum > 128.0 { 1u8 } else { 0u8 };
        
        h ^= bw as u64;
        h = h.wrapping_mul(0x100000001b3);
        
        i += step;
    }
    h
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    model: Arc<Mutex<AppModel>>,
    settings: Settings,
    show_settings: bool,
    /// true once when user opens Settings: try to fetch models from API
    settings_fetch_models_pending: bool,
    last_errors: std::collections::BTreeMap<usize, String>,
    /// Empty = use built-in fallback list until Refresh succeeds
    gemini_models: Vec<GeminiModel>,

    /// Fullscreen drag-to-select overlay (one at a time).
    crop_session: Option<Arc<Mutex<CropOverlayState>>>,
    crop_finish: Arc<Mutex<Option<CropOutcome>>>,

    capture: Arc<dyn FrameSource>,

    /// Local OCR engine (Offline)
    windows_ocr: Arc<WindowsOcr>,

    /// Text-only translator via selected provider (Gemini/Groq/Ollama)
    translator: Option<Arc<dyn Translator>>,

    // Background processing
    bg_tx: mpsc::Sender<BgResult>,
    bg_rx: mpsc::Receiver<BgResult>,
    slot_busy: Vec<bool>,
    slot_processing: Vec<bool>,
    slot_status: Vec<String>,

    /// Hash of the last captured frame per slot — used to skip API calls
    /// when the screen content hasn't changed.
    last_frame_hash: Vec<u64>,

    /// Available displays for capturing (ID, Label)
    available_screens: Vec<(u32, String)>,

    /// Cache for (smart_hash, source_lang, target_lang) → (ocr_text, translated_text)
    translation_cache: Arc<Mutex<std::collections::HashMap<(u64, Option<String>, String), (String, String)>>>,

    /// Cache for OCR text hash → translated_text.
    /// Catches cases where the same text appears with different pixel content
    /// (e.g., cursor blink, slight background variation) without re-calling the API.
    text_translation_cache: Arc<Mutex<std::collections::HashMap<(u64, Option<String>, String), String>>>,

    /// Per-slot cache of the raw HWND (as isize) for the overlay window.
    /// We call FindWindowW every frame (cheap) and re-apply SetLayeredWindowAttributes
    /// only when the HWND changes — this handles window recreation after model switch,
    /// overlay mode toggle, or any other event that causes egui to recreate the child window.
    /// 0 = not yet found.
    overlay_hwnd_cache: Vec<Arc<std::sync::atomic::AtomicIsize>>,

    /// Last known (source_lang, target_lang) per slot.
    /// When this changes we invalidate caches and reset frame hash to force retranslation.
    slot_last_langs: Vec<(Option<String>, String)>,
    
    /// Channel to signal error dismissal from the error viewport
    error_dismiss_tx: mpsc::Sender<()>,
    error_dismiss_rx: mpsc::Receiver<()>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ── Font setup: multi-script support ─────────────────────────────
        // egui's default fonts cover Latin, Cyrillic, and Greek.
        // We add the following fallbacks so every translated script renders:
        //   • Thai          → NotoSansThai (embedded, guaranteed)
        //   • CJK           → Microsoft YaHei / MS Gothic / Malgun Gothic (Windows system)
        //   • Arabic/Hebrew → Arial / Tahoma (Windows system)
        //   • Devanagari    → Nirmala UI / Mangal (Windows system)
        let mut fonts = egui::FontDefinitions::default();

        // 1. Embedded Thai font (always available)
        fonts.font_data.insert(
            "noto_sans_thai".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../../assets/NotoSansThai.ttf"
            ))),
        );

        // 2. Windows system fonts loaded at runtime
        //    We try each path; missing fonts are silently skipped.
        let system_fonts: &[(&str, &str)] = &[
            // CJK — Chinese (Simplified), Japanese, Korean
            ("msyh",    r"C:\Windows\Fonts\msyh.ttc"),     // Microsoft YaHei  (zh)
            ("msyh",    r"C:\Windows\Fonts\msyhbd.ttc"),
            ("msgoth",  r"C:\Windows\Fonts\msgothic.ttc"),  // MS Gothic         (ja)
            ("malgun",  r"C:\Windows\Fonts\malgun.ttf"),    // Malgun Gothic     (ko)
            ("malgunbd",r"C:\Windows\Fonts\malgunbd.ttf"),
            // Arabic, Hebrew, and wide Latin coverage
            ("arial",   r"C:\Windows\Fonts\arial.ttf"),
            ("tahoma",  r"C:\Windows\Fonts\tahoma.ttf"),
            // Devanagari (Hindi, Nepali, Marathi) + other South-Asian scripts
            ("nirmala", r"C:\Windows\Fonts\Nirmala.ttf"),
            ("nirmalab",r"C:\Windows\Fonts\NirmalaB.ttf"),
            ("mangal",  r"C:\Windows\Fonts\mangal.ttf"),
            // Fallback Unicode catch-all (Office installs)
            ("arialuni",r"C:\Windows\Fonts\ARIALUNI.TTF"),
        ];

        let mut loaded: Vec<String> = Vec::new();
        for (key, path) in system_fonts {
            if loaded.contains(&key.to_string()) {
                continue; // skip duplicate keys
            }
            if let Ok(data) = std::fs::read(path) {
                fonts.font_data.insert(
                    (*key).to_owned(),
                    Arc::new(egui::FontData::from_owned(data)),
                );
                loaded.push(key.to_string());
            }
        }

        // 3. Register all fonts as fallbacks (Thai first, then system fonts)
        let fallback_order = {
            let mut v = vec!["noto_sans_thai".to_owned()];
            v.extend(loaded.iter().cloned());
            v
        };
        for family_key in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(family) = fonts.families.get_mut(&family_key) {
                family.extend(fallback_order.clone());
            }
        }

        cc.egui_ctx.set_fonts(fonts);

        let settings = load_settings().unwrap_or_default();

        let translator = Self::create_translator(&settings);

        let (bg_tx, bg_rx) = mpsc::channel();
        let (err_tx, err_rx) = mpsc::channel();

        if settings.dark_mode {
            let mut visuals = egui::Visuals::dark();
            visuals.window_corner_radius = 6.0.into();
            visuals.widgets.noninteractive.corner_radius = 6.0.into();
            visuals.widgets.inactive.corner_radius = 6.0.into();
            visuals.widgets.hovered.corner_radius = 6.0.into();
            visuals.widgets.active.corner_radius = 6.0.into();
            visuals.widgets.open.corner_radius = 6.0.into();
            cc.egui_ctx.set_visuals(visuals);
        } else {
            let mut visuals = egui::Visuals::light();
            visuals.window_corner_radius = 6.0.into();
            visuals.widgets.noninteractive.corner_radius = 6.0.into();
            visuals.widgets.inactive.corner_radius = 6.0.into();
            visuals.widgets.hovered.corner_radius = 6.0.into();
            visuals.widgets.active.corner_radius = 6.0.into();
            visuals.widgets.open.corner_radius = 6.0.into();
            cc.egui_ctx.set_visuals(visuals);
        }

        Self {
            model: Arc::new(Mutex::new(AppModel::new_default())),
            settings,
            show_settings: false,
            settings_fetch_models_pending: false,
            last_errors: std::collections::BTreeMap::new(),
            gemini_models: Vec::new(),
            crop_session: None,
            crop_finish: Arc::new(Mutex::new(None)),
            capture: Arc::new(ScreenshotsCapture::new()),
            windows_ocr: Arc::new(WindowsOcr::new()),
            translator,
            bg_tx,
            bg_rx,
            slot_busy: vec![false],
            slot_processing: vec![false],
            slot_status: vec!["Idle".to_string()],
            last_frame_hash: vec![0],
            available_screens: screenshots::Screen::all()
                .unwrap_or_default()
                .into_iter()
                .map(|s| {
                    let w = s.display_info.width;
                    let h = s.display_info.height;
                    let label = if s.display_info.is_primary {
                        format!("Primary {}x{} (Screen {})", w, h, s.display_info.id)
                    } else {
                        format!("{}x{} (Screen {})", w, h, s.display_info.id)
                    };
                    (s.display_info.id, label)
                })
                .collect(),
            translation_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
            text_translation_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
            overlay_hwnd_cache: vec![Arc::new(std::sync::atomic::AtomicIsize::new(0))],
            slot_last_langs: vec![(None, String::new())],
            error_dismiss_tx: err_tx,
            error_dismiss_rx: err_rx,
        }
    }

    fn fallback_gemini_models() -> Vec<GeminiModel> {
        vec![
            GeminiModel {
                id: "gemini-2.5-flash".to_string(),
                display_name: "Gemini 2.5 Flash".to_string(),
            },
            GeminiModel {
                id: "gemini-2.0-flash".to_string(),
                display_name: "Gemini 2.0 Flash".to_string(),
            },
            GeminiModel {
                id: "gemini-2.0-flash-lite".to_string(),
                display_name: "Gemini 2.0 Flash Lite".to_string(),
            },
            GeminiModel {
                id: "gemini-1.5-flash".to_string(),
                display_name: "Gemini 1.5 Flash".to_string(),
            },
        ]
    }

    /// Choices for dropdown: API list if loaded, else fallback; ensure current setting appears.
    fn model_choices(&self) -> Vec<GeminiModel> {
        let mut v = if self.gemini_models.is_empty() {
            Self::fallback_gemini_models()
        } else {
            self.gemini_models.clone()
        };
        let cur = self.settings.gemini_model.trim();
        if !cur.is_empty() && !v.iter().any(|m| m.id == cur) {
            v.insert(
                0,
                GeminiModel {
                    id: cur.to_string(),
                    display_name: format!("{cur} (current)"),
                },
            );
        }
        v
    }

    fn language_options() -> Vec<(&'static str, &'static str)> {
        vec![
            // ── Detection ─────────────────────────────────────────────────
            ("Auto-detect", ""),
            // ── East / Southeast Asia ──────────────────────────────────────
            ("Chinese Simplified (zh-Hans)", "zh-Hans"),
            ("Chinese Traditional (zh-Hant)", "zh-Hant"),
            ("Japanese (ja)",                 "ja"),
            ("Korean (ko)",                   "ko"),
            ("Thai (th)",                     "th"),
            ("Vietnamese (vi)",               "vi"),
            ("Indonesian (id)",               "id"),
            ("Malay (ms)",                    "ms"),
            ("Filipino / Tagalog (fil)",      "fil"),
            ("Burmese (my)",                  "my"),
            ("Khmer (km)",                    "km"),
            ("Lao (lo)",                      "lo"),
            // ── South Asia ────────────────────────────────────────────────
            ("Hindi (hi)",                    "hi"),
            ("Bengali (bn)",                  "bn"),
            ("Tamil (ta)",                    "ta"),
            ("Telugu (te)",                   "te"),
            ("Urdu (ur)",                     "ur"),
            ("Nepali (ne)",                   "ne"),
            ("Sinhala (si)",                  "si"),
            // ── Middle East / Central Asia ────────────────────────────────
            ("Arabic (ar)",                   "ar"),
            ("Persian / Farsi (fa)",          "fa"),
            ("Turkish (tr)",                  "tr"),
            ("Hebrew (he)",                   "he"),
            // ── Europe — Western ──────────────────────────────────────────
            ("English (en)",                  "en"),
            ("French (fr)",                   "fr"),
            ("German (de)",                   "de"),
            ("Spanish (es)",                  "es"),
            ("Portuguese (pt)",               "pt"),
            ("Italian (it)",                  "it"),
            ("Dutch (nl)",                    "nl"),
            ("Swedish (sv)",                  "sv"),
            ("Norwegian (no)",                "no"),
            ("Danish (da)",                   "da"),
            ("Finnish (fi)",                  "fi"),
            ("Polish (pl)",                   "pl"),
            ("Romanian (ro)",                 "ro"),
            ("Czech (cs)",                    "cs"),
            ("Hungarian (hu)",                "hu"),
            ("Greek (el)",                    "el"),
            // ── Europe — Eastern / Cyrillic ───────────────────────────────
            ("Russian (ru)",                  "ru"),
            ("Ukrainian (uk)",                "uk"),
            ("Bulgarian (bg)",                "bg"),
            ("Serbian (sr)",                  "sr"),
            ("Croatian (hr)",                 "hr"),
            // ── Americas / African ────────────────────────────────────────
            ("Swahili (sw)",                  "sw"),
            ("Afrikaans (af)",                "af"),
        ]
    }

    fn ui_slot(&mut self, ui: &mut egui::Ui, slot_idx: usize) -> bool {
        let mut do_crop = false;
        let mut should_remove = false;

        let frame = egui::Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(8.0)
            .inner_margin(10.0);

        frame.show(ui, |ui| {
            ui.set_min_width(500.0);
            
            // --- HEADER ROW ---
            ui.horizontal(|ui| {
                ui.heading(format!("Region {}", slot_idx + 1));
                
                let mut model = self.model.lock();
                let slot = &mut model.slots[slot_idx];
                
                ui.checkbox(&mut slot.enabled, "Active").on_hover_text("Enable or disable this translation region");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if slot_idx > 0 {
                        if ui.button("🗑").on_hover_text("Delete this region").clicked() {
                            should_remove = true;
                        }
                    }
                    
                    if ui.button("🗺 Select Area")
                        .on_hover_text("Drag to select a new area on the screen")
                        .clicked() 
                    {
                        do_crop = true;
                    }
                });
            });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // --- SETTINGS ROW (Screen & Refresh) ---
            ui.horizontal(|ui| {
                ui.label("🖥 Screen:");
                let mut model = self.model.lock();
                let slot = &mut model.slots[slot_idx];

                egui::ComboBox::from_id_salt(format!("disp_sel_{}", slot_idx))
                    .selected_text({
                        self.available_screens.iter()
                            .find(|(id, _)| *id == slot.display_id)
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| "Primary".to_string())
                    })
                    .show_ui(ui, |ui| {
                        for (id, name) in &self.available_screens {
                            ui.selectable_value(&mut slot.display_id, *id, name);
                        }
                    });

                ui.add_space(20.0);
                ui.label("⏱ Refresh:");
                ui.add(egui::DragValue::new(&mut slot.refresh_ms).speed(10.0).suffix("ms"))
                    .on_hover_text("How often to check for screen changes");
            });

            ui.add_space(8.0);

            // --- TRANSLATION ROW (From/To) ---
            ui.horizontal(|ui| {
                let mut model = self.model.lock();
                let slot = &mut model.slots[slot_idx];

                ui.label("🌐 From:");
                let mut src = slot.source_lang.as_ref().map(|l| l.0.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt(format!("src_{slot_idx}"))
                    .selected_text(
                        Self::language_options().iter()
                            .find(|(_, code)| code.to_string() == src)
                            .map(|(name, _)| *name).unwrap_or("Auto Detection"),
                    )
                    .show_ui(ui, |ui| {
                        for (name, code) in Self::language_options() {
                            ui.selectable_value(&mut src, code.to_string(), name);
                        }
                    });
                slot.source_lang = if src.is_empty() { None } else { Some(LanguageTag(src)) };

                ui.add_space(10.0);
                ui.label("➡️ To:");
                let mut tgt = slot.target_lang.0.clone();
                egui::ComboBox::from_id_salt(format!("tgt_{slot_idx}"))
                    .selected_text(
                        Self::language_options().iter()
                            .find(|(_, code)| code.to_string() == tgt)
                            .map(|(name, _)| *name).unwrap_or("Thai (th)"),
                    )
                    .show_ui(ui, |ui| {
                        for (name, code) in Self::language_options() {
                            if code.is_empty() { continue; }
                            ui.selectable_value(&mut tgt, code.to_string(), name);
                        }
                    });
                slot.target_lang = LanguageTag(tgt);
            });

            ui.add_space(8.0);

            // --- VIEW OPTIONS ROW ---
            ui.horizontal(|ui| {
                let mut model = self.model.lock();
                let slot = &mut model.slots[slot_idx];

                ui.checkbox(&mut slot.show_frame, "👁 Show Frame Box").on_hover_text("Show a green border around the captured area");
                ui.add_space(10.0);
                ui.checkbox(&mut slot.overlay_mode, "📺 Overlay Mode").on_hover_text("Show translated text directly over the original text on your screen");
                ui.add_space(20.0);
                
                let popup_btn_text = if slot.popup_open { "💬 Close Popup" } else { "💬 Open Popup" };
                if ui.button(popup_btn_text).on_hover_text("Toggle the in-game translation result window").clicked() {
                    slot.popup_open = !slot.popup_open;
                }
            });

            ui.add_space(8.0);

            // --- ADVANCED / POSITION ROW ---
            egui::CollapsingHeader::new("🔍 Manual Position Adjustment")
                .id_salt(format!("manual_adj_{slot_idx}"))
                .default_open(false)
                .show(ui, |ui| {
                    let mut model = self.model.lock();
                    let slot = &mut model.slots[slot_idx];

                    if slot.rect.is_none() {
                        slot.rect = Some(Rect { x: 0, y: 0, w: 400, h: 200 });
                    }
                    if let Some(r) = slot.rect.as_mut() {
                        ui.horizontal(|ui| {
                            ui.label("X:"); ui.add(egui::DragValue::new(&mut r.x));
                            ui.add_space(8.0);
                            ui.label("Y:"); ui.add(egui::DragValue::new(&mut r.y));
                            ui.add_space(8.0);
                            ui.label("W:"); ui.add(egui::DragValue::new(&mut r.w));
                            ui.add_space(8.0);
                            ui.label("H:"); ui.add(egui::DragValue::new(&mut r.h));
                        });
                    }
                });

            // --- RESULTS AREA ---
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // Translation display removed from main UI as requested.
            // Users can see it in Popup or Overlay Mode.
            ui.horizontal(|ui| {
                if self.slot_processing[slot_idx] || self.slot_busy[slot_idx] {
                    ui.add(egui::Spinner::new().size(12.0));
                } else {
                    ui.label("💤");
                }
                ui.label(egui::RichText::new(&self.slot_status[slot_idx]).size(13.0).strong());
            });
        });

        if do_crop {
            let display_id = self.model.lock().slots[slot_idx].display_id;
            match CropOverlayState::start(slot_idx, display_id, ui.ctx()) {
                Ok(st) => {
                    *self.crop_finish.lock() = None;
                    self.crop_session = Some(Arc::new(Mutex::new(st)));
                    self.last_errors.clear();
                }
                Err(e) => {
                    self.last_errors.insert(999, format!("{e:#}"));
                }
            }
        }

        should_remove
    }

    fn ui_popups(&mut self, ctx: &egui::Context) {
        let model_slots: Vec<_> = { self.model.lock().slots.clone() };
        for slot in model_slots {
            if !slot.popup_open {
                continue;
            }
            let title = format!("Region {} (Popup)", slot.id.0 + 1);
            let viewport_id = egui::ViewportId::from_hash_of(format!("popup_{}", slot.id.0));
            let model_arc = self.model.clone();
            let slot_id = slot.id.0;
            
            ctx.show_viewport_immediate(
                viewport_id,
                egui::ViewportBuilder::default()
                    .with_title(&title)
                    .with_inner_size([400.0, 200.0])
                    .with_always_on_top(),
                move |ctx, class| {
                    if ctx.input(|i| i.viewport().close_requested()) {
                        let mut m = model_arc.lock();
                        // Find the slot by checking length, but we can just use slot_id if it's within bounds.
                        if slot_id < m.slots.len() {
                            m.slots[slot_id].popup_open = false;
                        }
                    }
                    
                    let show_content = |ui: &mut egui::Ui| {
                        if !slot.last_ocr_text.is_empty() {
                            ui.label("OCR:");
                            ui.monospace(&slot.last_ocr_text);
                        }
                        ui.separator();
                        ui.label("Translation:");
                        if slot.last_trans_lines.is_empty() {
                            ui.monospace("(waiting...)");
                        } else {
                            for line in &slot.last_trans_lines {
                                ui.label(line);
                            }
                        }
                    };

                    if matches!(class, egui::ViewportClass::Embedded) {
                        egui::Window::new(&title).show(ctx, show_content);
                    } else {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            egui::ScrollArea::vertical().show(ui, show_content);
                        });
                    }
                }
            );
        }
    }

    fn ui_frames(&mut self, ctx: &egui::Context) {
        let model_slots: Vec<_> = { self.model.lock().slots.clone() };
        for slot in model_slots {
            if !slot.show_frame && !slot.overlay_mode {
                continue;
            }
            let Some(r) = slot.rect else {
                continue;
            };

            let title = format!("Frame Overlay {}", slot.id.0 + 1);
            let viewport_id = egui::ViewportId::from_hash_of(format!("frame_overlay_{}", slot.id.0));
            let model_arc = self.model.clone();
            let slot_id = slot.id.0;

            // Ensure the Vec is large enough (can happen on first run before Add Region)
            while self.overlay_hwnd_cache.len() <= slot_id {
                self.overlay_hwnd_cache.push(Arc::new(std::sync::atomic::AtomicIsize::new(0)));
            }
            // Clone the Arc so the closure can own it without needing &mut self
            let hwnd_cache = self.overlay_hwnd_cache[slot_id].clone();

            ctx.show_viewport_immediate(
                viewport_id,
                egui::ViewportBuilder::default()
                    .with_title(&title)
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_always_on_top()
                    .with_mouse_passthrough(true)
                    .with_active(false) // CRITICAL: Prevent focus theft and black boxes
                    .with_inner_size(egui::vec2(r.w as f32, r.h as f32))
                    .with_position(egui::pos2(r.x as f32, r.y as f32)),
                move |ctx, class| {
                    if matches!(class, egui::ViewportClass::Embedded) {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.label("Frame Viewer (Embedded)");
                        });
                        return;
                    }

                    // Paint directly on the transparent GPU-cleared surface.
                    let full_rect = ctx.screen_rect();
                    let painter = ctx.layer_painter(egui::LayerId::background());

                    {
                        let m = model_arc.lock();
                        if slot_id < m.slots.len() {
                            let slot = &m.slots[slot_id];
                            let show_overlay = slot.overlay_mode && !slot.last_translation.is_empty();
                            let show_border  = slot.show_frame;
                            let ocr_lines    = slot.last_ocr_lines.clone();
                            let trans_lines  = slot.last_trans_lines.clone();
                            let fallback_text = slot.last_translation.clone();
                            drop(m);

                            if show_overlay {
                                // ── Positional overlay ────────────────────────────────────────
                                // Background is TRANSPARENT. Only the small dark rect behind
                                // each translated line is drawn, matching the original text position.
                                let has_positions = !ocr_lines.is_empty();

                                if has_positions {
                                    // Max width for text wrapping = region width
                                    let max_text_w = full_rect.width() - 8.0;
                                    
                                    // Track the bottom position of the last rendered line to avoid overlaps
                                    let mut last_bottom_y = full_rect.top();

                                    for (idx, ocr_line) in ocr_lines.iter().enumerate() {
                                        let trans = trans_lines
                                            .get(idx)
                                            .map(|s| s.as_str())
                                            .unwrap_or("");
                                        if trans.trim().is_empty() { continue; }

                                        // Font size ~90% of OCR line height, clamped sensibly
                                        let font_size = (ocr_line.h * 0.90).clamp(11.0, 26.0);

                                        // Wrap text within region width
                                        let wrap_width = (max_text_w - ocr_line.x + full_rect.left()).max(100.0);
                                        let galley = ctx.fonts(|f| {
                                            f.layout(
                                                trans.to_string(),
                                                egui::FontId::proportional(font_size),
                                                egui::Color32::WHITE,
                                                wrap_width,
                                            )
                                        });

                                        // ── Collision Avoidance ──────────────────────────────────────
                                        // Ensure this line starts AFTER the previous line's background
                                        let mut start_y = ocr_line.y;
                                        if start_y < last_bottom_y {
                                            start_y = last_bottom_y + 1.0; // Small 1px gap
                                        }

                                        // Background covers the ENTIRE OCR line area to fully hide original text
                                        let bg_w = ocr_line.w.max(galley.size().x + 10.0).min(wrap_width + 10.0);
                                        let bg_h = ocr_line.h.max(galley.size().y + 4.0);
                                        let bg = egui::Rect::from_min_size(
                                            egui::pos2(ocr_line.x - 2.0, start_y - 1.0),
                                            egui::vec2(bg_w + 4.0, bg_h + 2.0),
                                        );
                                        
                                        // Update last_bottom_y for the next iteration
                                        last_bottom_y = bg.max.y;

                                        // Fully opaque dark background — NOT pure black (color-keyed)
                                        painter.rect_filled(
                                            bg,
                                            3.0,
                                            egui::Color32::from_rgba_unmultiplied(18, 18, 30, 255),
                                        );

                                        // Center text vertically within the calculated background box
                                        let text_y = start_y + (bg_h - galley.size().y) / 2.0;
                                        let text_pos = egui::pos2(ocr_line.x, text_y);
                                        painter.galley(text_pos, galley, egui::Color32::WHITE);
                                    }

                                    // Render any extra translated lines below everything else
                                    if trans_lines.len() > ocr_lines.len() {
                                        let last = ocr_lines.last().unwrap();
                                        let mut y = last_bottom_y + 4.0;
                                        for extra in &trans_lines[ocr_lines.len()..] {
                                            if extra.trim().is_empty() { continue; }
                                            let wrap_width = (full_rect.width() - last.x + full_rect.left() - 8.0).max(100.0);
                                            let galley = ctx.fonts(|f| {
                                                f.layout(
                                                    extra.clone(),
                                                    egui::FontId::proportional(14.0),
                                                    egui::Color32::WHITE,
                                                    wrap_width,
                                                )
                                            });
                                            let pos = egui::pos2(last.x, y);
                                            let bg = egui::Rect::from_min_size(
                                                pos - egui::vec2(5.0, 3.0),
                                                galley.size() + egui::vec2(10.0, 6.0),
                                            );
                                            painter.rect_filled(bg, 3.0, egui::Color32::from_rgba_unmultiplied(18, 18, 30, 255));
                                            let line_h = galley.size().y;
                                            painter.galley(pos, galley, egui::Color32::WHITE);
                                            y += line_h + 4.0;
                                        }
                                    }
                                } else {
                                    // Fallback: no position info (cache hit) — stack lines vertically centered
                                    let font_size = 16.0_f32;
                                    let mut y = full_rect.top() + 8.0;
                                    for line in fallback_text.lines() {
                                        if line.trim().is_empty() { continue; }
                                        let wrap_width = full_rect.width() - 16.0;
                                        let galley = ctx.fonts(|f| {
                                            f.layout(
                                                line.to_string(),
                                                egui::FontId::proportional(font_size),
                                                egui::Color32::WHITE,
                                                wrap_width,
                                            )
                                        });
                                        let x = (full_rect.center().x - galley.size().x / 2.0)
                                            .clamp(full_rect.left() + 4.0, full_rect.right() - 4.0);
                                        let pos = egui::pos2(x, y);
                                        let bg = egui::Rect::from_min_size(
                                            pos - egui::vec2(5.0, 3.0),
                                            galley.size() + egui::vec2(10.0, 6.0),
                                        );
                                        painter.rect_filled(bg, 3.0, egui::Color32::from_rgba_unmultiplied(18, 18, 30, 255));
                                        let line_h = galley.size().y;
                                        painter.galley(pos, galley, egui::Color32::WHITE);
                                        y += line_h + 4.0;
                                    }
                                }
                            }

                            // Green frame border (shown when Show Frame Box is ticked)
                            if show_border {
                                let stroke = egui::Stroke::new(2.5, egui::Color32::from_rgb(0, 255, 128));
                                painter.rect_stroke(full_rect, 0.0, stroke, egui::StrokeKind::Inside);
                            }
                        }
                    }

                    // Drag to reposition the capture region
                    ctx.input(|i| {
                        if i.pointer.primary_down() {
                            let delta = i.pointer.delta();
                            if delta != egui::Vec2::ZERO {
                                let mut m = model_arc.lock();
                                if slot_id < m.slots.len() {
                                    if let Some(rect) = m.slots[slot_id].rect.as_mut() {
                                        rect.x += delta.x as i32;
                                        rect.y += delta.y as i32;
                                    }
                                }
                            }
                        }
                    });

                    // Win32 Color Key Transparency
                    // FindWindowW is cheap (~0.1 ms); we call it every frame and compare
                    // with the cached HWND. SetLayeredWindowAttributes is only called when
                    // the HWND changes (window recreated after model switch / overlay toggle)
                    // or on first appearance. This is robust against all window recreation.
                    #[cfg(target_os = "windows")]
                    unsafe {
                        use std::ptr;
                        use windows::Win32::Foundation::COLORREF;
                        use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
                        let title_w: Vec<u16> = format!("Frame Overlay {}\0", slot_id + 1)
                            .encode_utf16()
                            .collect();
                        if let Ok(hwnd) = FindWindowW(
                            windows::core::PCWSTR(ptr::null()),
                            windows::core::PCWSTR(title_w.as_ptr()),
                        ) {
                            let raw = hwnd.0 as isize;
                            let cached = hwnd_cache.load(std::sync::atomic::Ordering::Relaxed);
                            if raw != 0 && raw != cached {
                                use windows::Win32::UI::WindowsAndMessaging::*;
                                
                                let mut style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                                style &= !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_SYSMENU.0);
                                SetWindowLongW(hwnd, GWL_STYLE, style as i32);

                                let mut ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                                ex_style |= WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0;
                                SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style as i32);

                                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_COLORKEY);
                                let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
                                
                                // Restore Display Affinity to hide overlay from capture
                                const WDA_EXCLUDEFROMCAPTURE: WINDOW_DISPLAY_AFFINITY = WINDOW_DISPLAY_AFFINITY(0x11);
                                let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
                                
                                hwnd_cache.store(raw, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                },
            );
        }
    }


    // -----------------------------------------------------------------------
    // Background processing: capture → compare → OCR+Translate (if changed)
    // -----------------------------------------------------------------------

    fn create_translator(settings: &Settings) -> Option<Arc<dyn Translator>> {
        match settings.provider {
            TranslationProvider::Gemini => GeminiTranslator::new(
                settings.gemini_api_key.clone(),
                settings.gemini_model.clone(),
            )
            .ok()
            .map(|t| Arc::new(t) as Arc<dyn Translator>),
            TranslationProvider::Groq => {
                GroqTranslator::new(settings.groq_api_key.clone(), settings.groq_model.clone())
                    .ok()
                    .map(|t| Arc::new(t) as Arc<dyn Translator>)
            }
            TranslationProvider::Ollama => OllamaTranslator::new(
                settings.ollama_url.clone(),
                settings.ollama_model.clone(),
            )
            .ok()
            .map(|t| Arc::new(t) as Arc<dyn Translator>),
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Parse a numbered translation response back into a Vec aligned to
    /// the original OCR lines.
    ///
    /// Gemini returns lines like:
    ///   "1. สวัสดี"
    ///   "2. เป็นอย่างไร"
    ///   "3. ฉันสบายดี"
    ///
    /// We extract the number and put the text at `result[number - 1]`.
    /// Any missing numbers get an empty string so positions stay aligned.
    /// If the response has no numbers at all (single-line or model ignored
    /// the instruction), fall back to plain line-split.
    fn parse_numbered_lines(raw: &str, ocr_count: usize) -> Vec<String> {
        /// Strip leading prefixes that AI models commonly add:
        /// "1. ", "1) ", "1: ", "- ", "* ", "• ", "n. ", etc.
        fn strip_prefix(s: &str) -> &str {
            let t = s.trim();
            if t.is_empty() { return t; }
            // Try stripping leading digits + punctuation (e.g. "1. ", "12) ", "3: ")
            let after_digits = t.trim_start_matches(|c: char| c.is_ascii_digit());
            if after_digits.len() < t.len() {
                // We stripped some digits — now skip punctuation and whitespace
                let after_punct = after_digits.trim_start_matches(|c: char| {
                    c == '.' || c == ')' || c == ':' || c == '-' || c == '>' || c.is_whitespace()
                });
                if !after_punct.is_empty() {
                    return after_punct;
                }
            }
            // Try stripping common bullet prefixes
            for prefix in &["- ", "* ", "• ", "· ", "> "] {
                if let Some(rest) = t.strip_prefix(prefix) {
                    return rest.trim();
                }
            }
            t
        }

        if ocr_count == 0 {
            return vec![];
        }

        // Preserve ALL lines (including empty ones) to maintain index alignment
        // with OCR line positions. DO NOT filter empty lines!
        let all_lines: Vec<String> = raw
            .lines()
            .map(|l| strip_prefix(l).to_string())
            .collect();

        // If line count matches exactly, return as-is (perfect alignment)
        if all_lines.len() == ocr_count {
            return all_lines;
        }

        // If AI returned more lines than OCR (e.g. added explanations),
        // try removing truly empty lines to see if it matches
        if all_lines.len() > ocr_count {
            let non_empty: Vec<String> = all_lines.iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect();
            if non_empty.len() == ocr_count {
                return non_empty;
            }
            // Still doesn't match — truncate to ocr_count
            return all_lines.into_iter().take(ocr_count).collect();
        }

        // AI returned fewer lines — pad with empty strings
        let mut result = all_lines;
        result.resize(ocr_count, String::new());
        result
    }

    fn tick_background(&mut self) {
        // 0. Handle error dismissal from the error viewport
        while let Ok(_) = self.error_dismiss_rx.try_recv() {
            self.last_errors.clear();
        }

        // 1. Drain results from background threads
        while let Ok(result) = self.bg_rx.try_recv() {
            match result {
                BgResult::Done {
                    slot_idx,
                    language_version,
                    ocr_text,
                    translated,
                    frame_hash,
                    ocr_lines,
                } => {
                    let current_version = self.model.lock().slots.get(slot_idx).map(|s| s.language_version).unwrap_or(0);
                    if language_version != current_version {
                        self.slot_busy[slot_idx] = false;
                        self.slot_processing[slot_idx] = false;
                        let mut model = self.model.lock();
                        if let Some(s) = model.slots.get_mut(slot_idx) {
                            s.next_tick_at_ms = 0;
                        }
                        return;
                    }
                    
                    self.slot_busy[slot_idx] = false;
                    self.slot_processing[slot_idx] = false;
                    self.slot_status[slot_idx] = "Idle".to_string();
                    self.last_frame_hash[slot_idx] = frame_hash;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    let slot = &mut model.slots[slot_idx];
                    slot.next_tick_at_ms = now.saturating_add(slot.refresh_ms.max(500));

                    // Only update UI if the OCR text actually changed
                    let new_ocr = ocr_text.trim();
                    let old_ocr = slot.last_ocr_text.trim();

                    if new_ocr.is_empty() {
                        // Screen is now empty — clear old translation to prevent ghosting
                        slot.last_ocr_text.clear();
                        slot.last_translation.clear();
                        slot.last_ocr_lines.clear();
                        slot.last_trans_lines.clear();
                    } else if new_ocr != old_ocr {
                        // Content changed — update with new translation
                        slot.last_ocr_text = ocr_text;
                        if !translated.trim().is_empty() {
                            let line_count = ocr_lines.len();
                            // Parse numbered response ("1. foo\n2. bar") back to
                            // a Vec aligned to OCR line indices.
                            slot.last_trans_lines =
                                Self::parse_numbered_lines(&translated, line_count);
                            slot.last_ocr_lines = ocr_lines;
                            slot.last_translation = translated;
                            slot.pending_text.clear();
                        }
                    } else {
                        // Content hasn't changed, but maybe we forced a re-translate 
                        // due to language change. We should still update the translation.
                        if !translated.trim().is_empty() {
                            let line_count = ocr_lines.len();
                            slot.last_trans_lines =
                                Self::parse_numbered_lines(&translated, line_count);
                            slot.last_ocr_lines = ocr_lines;
                            slot.last_translation = translated;
                        }
                    }
                    self.last_errors.remove(&slot_idx);
                }
                BgResult::Unchanged { slot_idx } => {
                    self.slot_busy[slot_idx] = false;
                    self.slot_status[slot_idx] = "Idle".to_string();
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    model.slots[slot_idx].next_tick_at_ms = now.saturating_add(model.slots[slot_idx].refresh_ms.max(200));
                }
                BgResult::HashChanged { slot_idx, new_hash } => {
                    self.slot_busy[slot_idx] = false;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    let slot = &mut model.slots[slot_idx];
                    
                    // If we've been unstable for > 500ms, STOP resetting the timer.
                    // This allows the background thread to eventually proceed with translation.
                    if now.saturating_sub(slot.stable_since_ms) < 500 {
                        slot.stable_since_ms = now;
                    }
                    slot.stable_hash = new_hash;
                    // Check very aggressively until stable (100ms)
                    slot.next_tick_at_ms = now.saturating_add(100);
                }
                BgResult::WaitingDebounce { slot_idx } => {
                    self.slot_busy[slot_idx] = false;
                    self.slot_status[slot_idx] = "Debouncing...".to_string();
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    // Keep aggressively checking until debounce passes
                    model.slots[slot_idx].next_tick_at_ms = now.saturating_add(100);
                }
                BgResult::CacheHit { slot_idx, language_version, ocr_text, translated, frame_hash } => {
                    let current_version = self.model.lock().slots.get(slot_idx).map(|s| s.language_version).unwrap_or(0);
                    if language_version != current_version {
                        self.slot_busy[slot_idx] = false;
                        self.slot_processing[slot_idx] = false;
                        let mut model = self.model.lock();
                        if let Some(s) = model.slots.get_mut(slot_idx) {
                            s.next_tick_at_ms = 0;
                        }
                        return;
                    }
                    self.slot_busy[slot_idx] = false;
                    self.slot_processing[slot_idx] = false;
                    self.slot_status[slot_idx] = "Idle".to_string();
                    self.last_frame_hash[slot_idx] = frame_hash;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    let slot = &mut model.slots[slot_idx];
                    slot.next_tick_at_ms = now.saturating_add(slot.refresh_ms.max(500));
                    slot.last_ocr_text = ocr_text;
                    if !translated.trim().is_empty() {
                        slot.last_translation = translated;
                        slot.pending_text.clear();
                    }
                }
                BgResult::Translating { slot_idx } => {
                    self.slot_processing[slot_idx] = true;
                    self.slot_status[slot_idx] = "Translating...".to_string();
                }
                BgResult::StatusUpdate { slot_idx, status } => {
                    self.slot_status[slot_idx] = status;
                }
                BgResult::Error { slot_idx, language_version, err } => {
                    let current_version = self.model.lock().slots.get(slot_idx).map(|s| s.language_version).unwrap_or(0);
                    if language_version != current_version {
                        self.slot_busy[slot_idx] = false;
                        self.slot_processing[slot_idx] = false;
                        let mut model = self.model.lock();
                        if let Some(s) = model.slots.get_mut(slot_idx) {
                            s.next_tick_at_ms = 0;
                        }
                        return;
                    }

                    self.slot_busy[slot_idx] = false;
                    self.slot_processing[slot_idx] = false;
                    self.slot_status[slot_idx] = "Error".to_string();
                    let now = Self::now_ms();

                    // Parse Gemini retryDelay (e.g. "27s") from 429 responses.
                    // Fall back to 30 s if not found.
                    let retry_ms: u64 = {
                        let re_delay = err
                            .split('"')
                            .skip_while(|s| *s != "retryDelay")
                            .nth(2) // value after key + colon token
                            .and_then(|s| s.trim_matches(|c: char| !c.is_ascii_digit() && c != '.').parse::<f64>().ok())
                            .unwrap_or(30.0);
                        (re_delay * 1000.0) as u64
                    };

                    {
                        let mut model = self.model.lock();
                        model.slots[slot_idx].next_tick_at_ms = now.saturating_add(retry_ms.max(10_000));
                    }

                    // Reset frame hash so the retry tick actually re-runs OCR+translate
                    // instead of hitting the Unchanged fast-path.
                    if slot_idx < self.last_frame_hash.len() {
                        self.last_frame_hash[slot_idx] = 0;
                    }

                    // Show a friendly error message instead of raw JSON.
                    let friendly = if err.contains("429") || err.contains("RESOURCE_EXHAUSTED") {
                        let secs = retry_ms / 1000;
                        format!("Region {}: API quota exceeded — retrying in {secs}s", slot_idx + 1)
                    } else {
                        // Strip raw JSON body, keep just the first meaningful line
                        let first_line = err.lines().next().unwrap_or(&err).trim().to_string();
                        format!("Region {}: {first_line}", slot_idx + 1)
                    };
                    self.last_errors.insert(slot_idx, friendly);
                    self.model.lock().running = false;
                }
            }
        }

        // 2. Snapshot the model to decide what to spawn
        let now = Self::now_ms();
        let snapshot = self.model.lock().clone();

        if !snapshot.running {
            return;
        }

        for (i, slot) in snapshot.slots.iter().enumerate() {
            if !slot.enabled || slot.rect.is_none() {
                continue;
            }

            // ── Language-change detection ────────────────────────────────
            // Grow tracking vec to match slot count (slots can be added at runtime)
            if self.slot_last_langs.len() <= i {
                self.slot_last_langs.resize(i + 1, (None, String::new()));
            }
            let cur_src = slot.source_lang.as_ref().map(|l| l.0.clone());
            let cur_tgt = slot.target_lang.0.clone();
            let lang_changed = self.slot_last_langs[i] != (cur_src.clone(), cur_tgt.clone());
            if lang_changed {
                self.slot_last_langs[i] = (cur_src, cur_tgt);
                // Reset frame hash → next tick will bypass Unchanged fast-path
                if i < self.last_frame_hash.len() {
                    self.last_frame_hash[i] = 0;
                }
                // Evict stale translation from frame cache for this slot
                self.translation_cache.lock().clear();
                // Flush text cache — translations are language-pair specific
                self.text_translation_cache.lock().clear();
                // Clear overlay so old-language text disappears immediately
                if let Some(m_slot) = self.model.lock().slots.get_mut(i) {
                    m_slot.language_version = m_slot.language_version.wrapping_add(1);
                    m_slot.last_trans_lines.clear();
                    m_slot.last_ocr_lines.clear();
                    m_slot.last_translation.clear();
                    m_slot.last_ocr_text.clear();
                    m_slot.next_tick_at_ms = 0; // Force immediate re-tick
                }
                // Also reset frame hash for this slot to force fresh OCR
                self.last_frame_hash[i] = 1;
                if let Some(m_slot) = self.model.lock().slots.get_mut(i) {
                    m_slot.stable_hash = 0; // Reset stability state
                    m_slot.stable_since_ms = 0;
                }
            }

            if self.slot_busy[i] || now < slot.next_tick_at_ms {
                continue;
            }

            // Mark busy & prevent re-trigger until result arrives
            self.slot_busy[i] = true;
            {
                let mut m = self.model.lock();
                m.slots[i].next_tick_at_ms = u64::MAX;
            }

            let rect = slot.rect.unwrap();
            let display_id = slot.display_id;
            let source_lang = slot.source_lang.clone();
            let target_lang = slot.target_lang.clone();
            let capture = self.capture.clone();
            let windows_ocr = self.windows_ocr.clone();
            let translator = self.translator.clone();
            let tx = self.bg_tx.clone();
            let prev_hash = self.last_frame_hash[i];

            // Debounce variables
            let stable_hash = slot.stable_hash;
            let stable_since_ms = slot.stable_since_ms;
            let debounce_dur_ms = 400; // Lowered from 800ms for faster response in games

            let cache_arc = self.translation_cache.clone();
            let text_cache_arc = self.text_translation_cache.clone();
            let language_version = slot.language_version;
            
            let tx_inner = tx.clone();
            let ctx_cp = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(BgResult::StatusUpdate { slot_idx: i, status: "Capturing...".to_string() });
                let tx_outer = tx.clone();
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let tx = tx_inner;
                    let result = (|| -> anyhow::Result<BgResult> {
                        let frame = capture.capture_rect(rect, display_id)?;
                        
                        let _ = tx.send(BgResult::StatusUpdate { slot_idx: i, status: "Hashing...".to_string() });
                        // ...
                        let hash = smart_hash(&frame.data);

                        let is_unstable = hash != stable_hash;
                        let unstable_dur = now.saturating_sub(stable_since_ms);

                        if is_unstable && unstable_dur < 500 {
                            return Ok(BgResult::HashChanged { slot_idx: i, new_hash: hash });
                        }
                        if unstable_dur < 500 && now.saturating_sub(stable_since_ms) < debounce_dur_ms {
                            return Ok(BgResult::WaitingDebounce { slot_idx: i });
                        }
                        if hash == prev_hash && prev_hash != 0 {
                            return Ok(BgResult::Unchanged { slot_idx: i });
                        }

                        let cache_key = (hash, source_lang.as_ref().map(|l| l.0.clone()), target_lang.0.clone());
                        {
                            let cache = cache_arc.lock();
                            if let Some((ocr, tra)) = cache.get(&cache_key) {
                                return Ok(BgResult::CacheHit {
                                    slot_idx: i,
                                    language_version,
                                    ocr_text: ocr.clone(),
                                    translated: tra.clone(),
                                    frame_hash: hash,
                                });
                            }
                        }

                        let _ = tx.send(BgResult::Translating { slot_idx: i });

                        let scale = 2u32;
                        let new_w = frame.width * scale;
                        let new_h = frame.height * scale;
                        let mut upscaled = vec![0u8; (new_w * new_h * 4) as usize];
                        for dst_y in 0..new_h {
                            for dst_x in 0..new_w {
                                let src_xf = dst_x as f32 / scale as f32;
                                let src_yf = dst_y as f32 / scale as f32;
                                let x0 = (src_xf as u32).min(frame.width - 1);
                                let y0 = (src_yf as u32).min(frame.height - 1);
                                let x1 = (x0 + 1).min(frame.width - 1);
                                let y1 = (y0 + 1).min(frame.height - 1);
                                let fx = src_xf - x0 as f32;
                                let fy = src_yf - y0 as f32;
                                let idx = |x: u32, y: u32| (y * frame.width + x) as usize * 4;
                                let dst_idx = (dst_y * new_w + dst_x) as usize * 4;
                                for c in 0..4 {
                                    let v00 = frame.data[idx(x0, y0) + c] as f32;
                                    let v10 = frame.data[idx(x1, y0) + c] as f32;
                                    let v01 = frame.data[idx(x0, y1) + c] as f32;
                                    let v11 = frame.data[idx(x1, y1) + c] as f32;
                                    let top = v00 + (v10 - v00) * fx;
                                    let bot = v01 + (v11 - v01) * fx;
                                    upscaled[dst_idx + c] = (top + (bot - top) * fy) as u8;
                                }
                            }
                        }
                        let preprocessed_frame = FrameRgba {
                            width: new_w,
                            height: new_h,
                            data: upscaled,
                        };

                        let _ = tx.send(BgResult::StatusUpdate { slot_idx: i, status: "OCR...".to_string() });
                        let mut ocr_lines = windows_ocr.recognize_lines(&preprocessed_frame, source_lang.as_ref())?;

                        let inv_scale = 1.0 / scale as f32;
                        for line in &mut ocr_lines {
                            line.x *= inv_scale; line.y *= inv_scale; line.w *= inv_scale; line.h *= inv_scale;
                        }

                        let ocr_text = ocr_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n").trim().to_string();

                        if ocr_text.is_empty() {
                            return Ok(BgResult::Done {
                                slot_idx: i,
                                language_version,
                                ocr_text: String::new(),
                                translated: String::new(),
                                frame_hash: hash,
                                ocr_lines: Vec::new(),
                            });
                        }

                        if source_lang.as_ref().map(|s| s.0 == target_lang.0).unwrap_or(false) {
                            let mut cache = cache_arc.lock();
                            cache.insert(cache_key, (ocr_text.clone(), ocr_text.clone()));
                            return Ok(BgResult::Done {
                                slot_idx: i, language_version, ocr_text: ocr_text.clone(), translated: ocr_text, frame_hash: hash, ocr_lines,
                            });
                        }

                        {
                            let text_hash = {
                                let mut h: u64 = 0xcbf29ce484222325;
                                for b in ocr_text.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); }
                                h
                            };
                            let text_cache_key = (text_hash, source_lang.as_ref().map(|l| l.0.clone()), target_lang.0.clone());
                            let cached = { let text_cache = text_cache_arc.lock(); text_cache.get(&text_cache_key).cloned() };
                            if let Some(cached_trans) = cached {
                                let mut cache = cache_arc.lock();
                                cache.insert(cache_key, (ocr_text.clone(), cached_trans.clone()));
                                return Ok(BgResult::Done {
                                    slot_idx: i, language_version, ocr_text, translated: cached_trans, frame_hash: hash, ocr_lines,
                                });
                            }
                        }

                        if let Some(t) = &translator {
                            let _ = tx.send(BgResult::StatusUpdate { slot_idx: i, status: "Translating (waiting for AI)...".to_string() });
                            let translated = t.translate(&ocr_text, source_lang.as_ref(), &target_lang)?;
                            {
                                let text_hash = {
                                    let mut h: u64 = 0xcbf29ce484222325;
                                    for b in ocr_text.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); }
                                    h
                                };
                                let text_cache_key = (text_hash, source_lang.as_ref().map(|l| l.0.clone()), target_lang.0.clone());
                                let mut frame_cache = cache_arc.lock();
                                frame_cache.insert(cache_key, (ocr_text.clone(), translated.clone()));
                                drop(frame_cache);
                                let mut text_cache = text_cache_arc.lock();
                                text_cache.insert(text_cache_key, translated.clone());
                            }
                            Ok(BgResult::Done { slot_idx: i, language_version, ocr_text, translated, frame_hash: hash, ocr_lines })
                        } else {
                            anyhow::bail!("Translation provider not configured or API key missing — click ⚙ to check settings");
                        }
                    })();
                    
                    match result {
                        Ok(res) => { let _ = tx.send(res); }
                        Err(e) => {
                            let _ = tx.send(BgResult::Error { 
                                slot_idx: i, 
                                language_version, 
                                err: format!("{e:#}") 
                            });
                        }
                    }
                }));

                // WAKE UP the UI thread
                ctx_cp.request_repaint();

                if res.is_err() {
                    let _ = tx_outer.send(BgResult::Error {
                        slot_idx: i,
                        language_version,
                        err: "Background thread panicked (system error)".to_string(),
                    });
                }
            });
        }
    }

    fn ui_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        if self.settings_fetch_models_pending && !self.settings.gemini_api_key.trim().is_empty() {
            self.settings_fetch_models_pending = false;
            match GeminiTranslator::list_models(&self.settings.gemini_api_key) {
                Ok(models) if !models.is_empty() => {
                    self.gemini_models = models;
                    self.last_errors.clear();
                }
                Ok(_) => {
                    self.last_errors.insert(999, "listModels returned empty (using built-in list)".to_string());
                }
                Err(e) => {
                    self.last_errors.insert(999, format!(
                        "{e:#}\n(using built-in model list; check API key or click Refresh)"
                    ));
                }
            }
        }

        // Clone data into shared state for the viewport closure
        let settings_arc: Arc<Mutex<Settings>> = Arc::new(Mutex::new(self.settings.clone()));
        let model_choices = self.model_choices();
        let close_flag: Arc<std::sync::atomic::AtomicBool> =
            Arc::new(std::sync::atomic::AtomicBool::new(false));
        let save_flag: Arc<std::sync::atomic::AtomicBool> =
            Arc::new(std::sync::atomic::AtomicBool::new(false));

        let close_flag2 = close_flag.clone();
        let save_flag2 = save_flag.clone();
        let settings_arc2 = settings_arc.clone();
        let viewport_id = egui::ViewportId::from_hash_of("settings_viewport");

        ctx.show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title("KTranslator - Settings")
                .with_inner_size([500.0, 350.0])
                .with_resizable(true)
                .with_always_on_top(),
            move |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close_flag2.store(true, std::sync::atomic::Ordering::Relaxed);
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut settings = settings_arc2.lock();

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
                    });
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

                            // ── Recommended models dropdown ──
                            let ollama_models: Vec<(&str, &str, &str)> = vec![
                                // ── Lightweight (CPU / Low VRAM) ──
                                ("qwen2.5:0.5b",     "Qwen 2.5 0.5B",     "⚡ Ultra-light, CPU OK (~1GB)"),
                                ("qwen2.5:1.5b",     "Qwen 2.5 1.5B",     "⚡ Very light, CPU OK (~2GB)"),
                                ("qwen2.5:3b",       "Qwen 2.5 3B",       "⚡ Light & capable (~3GB)"),
                                ("llama3.2:1b",      "Llama 3.2 1B",      "⚡ Meta ultra-light (~2GB)"),
                                ("llama3.2:3b",      "Llama 3.2 3B",      "⚡ Meta light (~3GB)"),
                                ("gemma2:2b",        "Gemma 2 2B",        "⚡ Google ultra-light (~2GB)"),
                                ("phi3:mini",        "Phi-3 Mini 3.8B",   "⚡ Microsoft light (~3GB)"),
                                // ── Medium (8GB VRAM) ──
                                ("qwen2.5:7b",       "Qwen 2.5 7B",       "🌟 Best for Asian languages (8GB)"),
                                ("qwen3:8b",         "Qwen 3 8B",         "🆕 Latest Qwen (8GB)"),
                                ("gemma2:9b",        "Gemma 2 9B",        "Google efficient (8GB)"),
                                ("aya-expanse:8b",   "Aya Expanse 8B",    "🌐 Multilingual specialist (8GB)"),
                                ("llama3.1:8b",      "Llama 3.1 8B",      "Meta versatile (8GB)"),
                                // ── Large (12-24GB VRAM) ──
                                ("qwen2.5:14b",      "Qwen 2.5 14B",      "🌟 High quality Asian (12GB)"),
                                ("qwen3:14b",        "Qwen 3 14B",        "🆕 Latest Qwen (12GB)"),
                                ("gemma2:27b",       "Gemma 2 27B",       "Google premium (20GB)"),
                                ("qwen2.5:32b",      "Qwen 2.5 32B",      "🏆 Premium quality (24GB)"),
                                ("qwen3:32b",        "Qwen 3 32B",        "🆕 Latest Qwen (24GB)"),
                                ("aya-expanse:32b",  "Aya Expanse 32B",   "🌐 Best multilingual (24GB)"),
                                // ── XL (48GB+ VRAM) ──
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

                            // Pull command helper
                            let pull_cmd = format!("ollama pull {}", settings.ollama_model);
                            ui.horizontal(|ui| {
                                ui.label("📋 Run this first:");
                                if ui.button(&pull_cmd).on_hover_text("Click to copy").clicked() {
                                    ui.ctx().copy_text(pull_cmd.clone());
                                }
                            });
                            ui.small("Make sure Ollama is running before clicking Save & Apply.");
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(4.0);

                    if ui.button(egui::RichText::new("💾 Save & Apply").size(16.0)).clicked() {
                        save_flag2.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            },
        );

        // Write back settings changes
        let updated_settings = settings_arc.lock().clone();
        self.settings = updated_settings;

        // Handle Save
        if save_flag.load(std::sync::atomic::Ordering::Relaxed) {
            if let Err(e) = save_settings(&self.settings) {
                self.last_errors.insert(999, format!("{e:#}"));
            } else {
                self.translator = Self::create_translator(&self.settings);
                self.last_errors.clear();
                self.show_settings = false;
            }
        }

        // Handle Close
        if close_flag.load(std::sync::atomic::Ordering::Relaxed) {
            self.show_settings = false;
        }
    }

    fn ui_error_popup(&mut self, ctx: &egui::Context) {
        if self.last_errors.is_empty() { return; }
        
        let viewport_id = egui::ViewportId::from_hash_of("error_popup");
        let tx = self.error_dismiss_tx.clone();
        let errors: Vec<String> = self.last_errors.values().cloned().collect();
        
        ctx.show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title("KTranslator - Error Report")
                .with_inner_size([450.0, 220.0])
                .with_always_on_top()
                .with_decorations(true)
                .with_resizable(false),
            move |ctx, _| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(10.0);
                        ui.heading(
                            egui::RichText::new("[!] System Error")
                                .color(egui::Color32::from_rgb(255, 80, 80))
                                .strong()
                        );
                        ui.add_space(10.0);
                        
                        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                            for err in &errors {
                                ui.label(egui::RichText::new(err).size(14.0));
                                ui.add_space(4.0);
                            }
                        });
                        
                        ui.add_space(15.0);
                        if ui.button(egui::RichText::new(" Dismiss All Errors ").size(16.0)).clicked() {
                            let _ = tx.send(());
                        }
                    });
                });
            }
        );
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Essential for transparent viewports: 
        // Force the GPU background clear color to be fully transparent.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Aggressively process results to prevent "Translating..." hang
        self.tick_background();

        // Force repaint if we are still busy translating
        let is_any_busy = self.slot_busy.iter().any(|&b| b);
        if is_any_busy {
            ctx.request_repaint();
        }

        self.ui_error_popup(ctx);

        if let Some(sess) = &self.crop_session {
            run_crop_viewport(ctx, sess.clone(), self.crop_finish.clone());
        }
        if let Some(out) = self.crop_finish.lock().take() {
            match out {
                CropOutcome::Done { slot, rect } => {
                    if let Some(s) = self.model.lock().slots.get_mut(slot) {
                        s.rect = Some(rect);
                    }
                }
                CropOutcome::Cancelled => {}
            }
            self.crop_session = None;
        }

        let mut required_height: f32 = 0.0;
        let mut required_width: f32 = 520.0;

        egui::TopBottomPanel::top("top_bar")
            .frame(egui::Frame::side_top_panel(ctx.style().as_ref()).inner_margin(8.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("🚀 KTranslator");
                    ui.add_space(12.0);
                    
                    let mut model = self.model.lock();
                    let running = &mut model.running;
                    
                    let (btn_text, btn_color) = if *running { 
                        ("⏹ Stop", egui::Color32::from_rgb(200, 50, 50)) 
                    } else { 
                        ("▶ Start", egui::Color32::from_rgb(50, 150, 50)) 
                    };

                    let button = egui::Button::new(egui::RichText::new(btn_text).color(egui::Color32::WHITE).strong())
                        .fill(btn_color)
                        .min_size(egui::vec2(80.0, 24.0));
                        
                    if ui.add(button).on_hover_text(if *running { "Stop translation loop" } else { "Start translation loop" }).clicked() {
                        *running = !*running;
                        if *running {
                            // Reset all timers to trigger immediately when starting manually
                            for slot in &mut model.slots {
                                slot.next_tick_at_ms = 0;
                            }
                            // Also clear errors when manually starting
                            self.last_errors.clear();
                        }
                    }
                    
                    ui.add_space(8.0);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let theme_icon = if self.settings.dark_mode { "🌙" } else { "🔆" };
                        if ui.button(theme_icon).on_hover_text("Toggle Dark/Light mode").clicked() {
                            self.settings.dark_mode = !self.settings.dark_mode;
                            let mut visuals = if self.settings.dark_mode { 
                                egui::Visuals::dark() 
                            } else { 
                                egui::Visuals::light() 
                            };
                            // Re-apply common rounding
                            visuals.window_corner_radius = 6.0.into();
                            visuals.widgets.noninteractive.corner_radius = 6.0.into();
                            visuals.widgets.inactive.corner_radius = 6.0.into();
                            visuals.widgets.hovered.corner_radius = 6.0.into();
                            visuals.widgets.active.corner_radius = 6.0.into();
                            visuals.widgets.open.corner_radius = 6.0.into();
                            ctx.set_visuals(visuals);
                            let _ = save_settings(&self.settings);
                        }

                        if ui.button("⚙").on_hover_text("Open Settings").clicked() {
                            self.show_settings = true;
                            self.settings_fetch_models_pending = true;
                        }
                    });
                });
                required_height += ui.min_size().y;
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let slot_count = self.model.lock().slots.len();
            
            let mut remove_idx = None;
            let content_resp = ui.vertical(|ui| {
                for i in 0..slot_count {
                    if self.ui_slot(ui, i) {
                        remove_idx = Some(i);
                    }
                    ui.add_space(8.0);
                }

                ui.add_space(8.0);
                if ui.button("➕ Add Region").clicked() {
                    let mut model = self.model.lock();
                    model.add_slot();
                    self.slot_busy.push(false);
                    self.slot_processing.push(false);
                    self.slot_status.push("Idle".to_string());
                    self.last_frame_hash.push(0);
                    self.overlay_hwnd_cache.push(Arc::new(std::sync::atomic::AtomicIsize::new(0)));
                }

                ui.add_space(8.0);
                ui.separator();
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("💡 Tip: If games don't translate, use 'Borderless Windowed' mode.").small().weak());
                });
            });

            required_height += content_resp.response.rect.height();
            // Pin the width to prevent the feedback loop growth bug
            required_width = 560.0;
            // Add padding for the window bottom
            required_height += 40.0;

            if let Some(idx) = remove_idx {
                let mut model = self.model.lock();
                model.slots.remove(idx);
                self.slot_busy.remove(idx);
                self.slot_processing.remove(idx);
                self.slot_status.remove(idx);
                self.last_frame_hash.remove(idx);
                if idx < self.overlay_hwnd_cache.len() {
                    self.overlay_hwnd_cache.remove(idx);
                }

                // Re-align Region IDs so they match array index
                for (i, slot) in model.slots.iter_mut().enumerate() {
                    slot.id.0 = i;
                }
            }
        });

        // Request resize if the current window height is different
        let current_size = ctx.screen_rect().size();
        if (current_size.y - required_height).abs() > 2.0 || (current_size.x - required_width).abs() > 2.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                required_width,
                required_height,
            )));
        }

        self.ui_settings(ctx);
        self.ui_popups(ctx);
        self.ui_frames(ctx);

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
