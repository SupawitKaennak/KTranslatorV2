use std::sync::{mpsc, Arc};

use eframe::egui;
use parking_lot::Mutex;

use crate::{
    adapters::{
        capture::screenshots_capture::ScreenshotsCapture,
        ocr::windows_ocr::WindowsOcr,
        translate::{
            create_translator,
            gemini::{GeminiModel, GeminiTranslator},
        },
    },
    core::{
        coordinator::BackgroundCoordinator,
        model::AppModel,
        ports::{FrameSource, Translator},
        worker::SlotRuntimeState,
    },
    infra::{
        settings::{load_settings, save_settings, Settings},
        win32,
    },
    ui::{
        components::{
            settings_ui::show_settings_window,
            slot_ui::render_slot_item,
        },
        crop_overlay::{run_crop_viewport, CropOutcome, CropOverlayState},
    },
};


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
    translator: Option<Arc<dyn Translator + Send + Sync>>,

    // Background processing
    coordinator: BackgroundCoordinator,
    slots_runtime: Vec<SlotRuntimeState>,

    /// Available displays for capturing (ID, Label)
    available_screens: Vec<(u32, String)>,

    /// Cache for (smart_hash, source_lang, target_lang) → (ocr_text, translated_text)
    translation_cache: Arc<Mutex<std::collections::HashMap<(u64, Option<String>, String), (String, String)>>>,

    /// Cache for OCR text hash → translated_text.
    /// Catches cases where the same text appears with different pixel content
    /// (e.g., cursor blink, slight background variation) without re-calling the API.
    text_translation_cache: Arc<Mutex<std::collections::HashMap<(u64, Option<String>, String), String>>>,

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

        let translator = create_translator(&settings);

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

        let coordinator = BackgroundCoordinator::new();

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
            coordinator,
            slots_runtime: vec![SlotRuntimeState::new()],
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
            while self.slots_runtime.len() <= slot_id {
                self.slots_runtime.push(SlotRuntimeState::new());
            }
            // Clone the Arc so the closure can own it without needing &mut self
            let hwnd_cache = self.slots_runtime[slot_id].overlay_hwnd.clone();

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
                    let title = format!("Frame Overlay {}", slot_id + 1);
                    if let Some(raw) = win32::find_window(&title) {
                        let cached = hwnd_cache.load(std::sync::atomic::Ordering::Relaxed);
                        if raw != cached {
                            win32::apply_overlay_attributes(raw);
                            hwnd_cache.store(raw, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                },
            );
        }
    }


    // -----------------------------------------------------------------------
    // Background processing: capture → compare → OCR+Translate (if changed)
    // -----------------------------------------------------------------------



    fn tick_background(&mut self) {
        // 1. Process pending signals from popups/error window
        while let Ok(_) = self.error_dismiss_rx.try_recv() {
            self.last_errors.clear();
        }

        // 2. Delegate background logic to coordinator
        self.coordinator.process_results(
            &self.model,
            &mut self.slots_runtime,
            &mut self.last_errors,
            &self.translation_cache,
            &self.text_translation_cache,
        );

        if let Some(translator) = &self.translator {
            self.coordinator.tick(
                &self.model,
                &mut self.slots_runtime,
                &self.capture,
                &self.windows_ocr,
                translator,
                &self.translation_cache,
                &self.text_translation_cache,
            );
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

        let model_choices = self.model_choices();
        let settings_arc = Arc::new(Mutex::new(self.settings.clone()));
        
        let resp = show_settings_window(ctx, settings_arc.clone(), model_choices);

        if resp.save_clicked {
            let updated = settings_arc.lock().clone();
            self.settings = updated;
            if let Err(e) = save_settings(&self.settings) {
                self.last_errors.insert(999, format!("{e:#}"));
            } else {
                self.translator = create_translator(&self.settings);
                self.last_errors.clear();
                self.show_settings = false;
            }
        }

        if resp.close_clicked {
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
        // Force continuous refresh so background translation results are picked up immediately
        // even when the main window is not focused.
        ctx.request_repaint();

        self.tick_background();
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
                    let mut model = self.model.lock();
                    let resp = render_slot_item(ui, i, &mut model, &self.slots_runtime[i], &self.available_screens);
                    if resp.should_remove {
                        remove_idx = Some(i);
                    }
                    if resp.do_crop {
                        let display_id = model.slots[i].display_id;
                        drop(model); // Release lock before calling start
                        match CropOverlayState::start(i, display_id, ui.ctx()) {
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
                    ui.add_space(8.0);
                }

                ui.add_space(8.0);
                if ui.button("➕ Add Region").clicked() {
                    let mut model = self.model.lock();
                    model.add_slot();
                    self.slots_runtime.push(SlotRuntimeState::new());
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
                if idx < self.slots_runtime.len() {
                    self.slots_runtime.remove(idx);
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
