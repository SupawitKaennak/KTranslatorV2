use std::sync::{mpsc, Arc};

use eframe::egui;
use parking_lot::Mutex;

use crate::{
    adapters::{
        capture::screenshots_capture::ScreenshotsCapture,
        ocr::windows_ocr::WindowsOcr,
        translate::gemini::{GeminiModel, GeminiTranslator},
    },
    core::{
        model::AppModel,
        ports::{FrameSource, OcrEngine, Translator},
        types::{LanguageTag, Rect},
    },
    infra::settings::{load_settings, save_settings, Settings},
    ui::crop_overlay::{run_crop_viewport, CropOutcome, CropOverlayState},
};

// ---------------------------------------------------------------------------
// Background-thread result messages
// ---------------------------------------------------------------------------

enum BgResult {
    /// Combined OCR + Translation completed successfully.
    Done {
        slot_idx: usize,
        ocr_text: String,
        translated: String,
        frame_hash: u64,
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
        ocr_text: String,
        translated: String,
        frame_hash: u64,
    },
    /// Background thread is now engaging Gemini or OCR (heavy work)
    Translating {
        slot_idx: usize,
    },
    /// An error occurred during OCR / Translation.
    Error {
        slot_idx: usize,
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
    last_error: Option<String>,
    /// Empty = use built-in fallback list until Refresh succeeds
    gemini_models: Vec<GeminiModel>,

    /// Fullscreen drag-to-select overlay (one at a time).
    crop_session: Option<Arc<Mutex<CropOverlayState>>>,
    crop_finish: Arc<Mutex<Option<CropOutcome>>>,

    capture: Arc<dyn FrameSource>,

    /// Local OCR engine (Offline)
    windows_ocr: Arc<WindowsOcr>,

    /// Text-only translator via Gemini API
    gemini_translator: Option<Arc<GeminiTranslator>>,

    // Background processing
    bg_tx: mpsc::Sender<BgResult>,
    bg_rx: mpsc::Receiver<BgResult>,
    slot_busy: Vec<bool>,
    /// Flags indicating the slot is currently performing an OCR/Translation API call (heavy work)
    slot_processing: Vec<bool>,

    /// Hash of the last captured frame per slot — used to skip API calls
    /// when the screen content hasn't changed.
    last_frame_hash: Vec<u64>,

    /// Available displays for capturing (ID, Label)
    available_screens: Vec<(u32, String)>,

    /// Cache for smart_hash -> (ocr_text, translated_text)
    translation_cache: Arc<Mutex<std::collections::HashMap<u64, (String, String)>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ── Register Thai font so egui can render ภาษาไทย ──
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "noto_sans_thai".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../../assets/NotoSansThai.ttf"
            ))),
        );
        // Add as fallback for both proportional and monospace families
        if let Some(family) = fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
        {
            family.push("noto_sans_thai".to_owned());
        }
        if let Some(family) = fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
        {
            family.push("noto_sans_thai".to_owned());
        }
        cc.egui_ctx.set_fonts(fonts);

        let settings = load_settings().unwrap_or_default();

        let gemini_translator = GeminiTranslator::new(
            settings.gemini_api_key.clone(),
            settings.gemini_model.clone(),
        )
        .ok()
        .map(Arc::new);

        let (bg_tx, bg_rx) = mpsc::channel();

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
            last_error: None,
            gemini_models: Vec::new(),
            crop_session: None,
            crop_finish: Arc::new(Mutex::new(None)),
            capture: Arc::new(ScreenshotsCapture),
            windows_ocr: Arc::new(WindowsOcr::new()),
            gemini_translator,
            bg_tx,
            bg_rx,
            slot_busy: vec![false],
            slot_processing: vec![false],
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
            ("Auto", ""),
            ("English (en)", "en"),
            ("Thai (th)", "th"),
            ("Japanese (ja)", "ja"),
            ("Chinese (zh)", "zh"),
            ("Korean (ko)", "ko"),
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

            let mut model = self.model.lock();
            let slot = &mut model.slots[slot_idx];

            if !slot.last_translation.is_empty() {
                ui.label(egui::RichText::new("Translated Text:").strong());
                ui.label(&slot.last_translation);
            } else if self.slot_processing[slot_idx] {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Thinking...");
                });
            } else {
                ui.weak("Waiting for content changes...");
            }
        });

        if do_crop {
            let display_id = self.model.lock().slots[slot_idx].display_id;
            match CropOverlayState::start(slot_idx, display_id, ui.ctx()) {
                Ok(st) => {
                    *self.crop_finish.lock() = None;
                    self.crop_session = Some(Arc::new(Mutex::new(st)));
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(format!("{e:#}")),
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
                        ui.monospace(if slot.last_translation.is_empty() {
                            "(waiting...)"
                        } else {
                            &slot.last_translation
                        });
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

            ctx.show_viewport_immediate(
                viewport_id,
                egui::ViewportBuilder::default()
                    .with_title(&title)
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_always_on_top()
                    .with_mouse_passthrough(true)
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
                    // Deliberately avoids CentralPanel which always fills panel_fill
                    // (white/dark) over the wgpu-transparent background.
                    let full_rect = ctx.screen_rect();
                    let painter = ctx.layer_painter(egui::LayerId::background());

                    {
                        let m = model_arc.lock();
                        if slot_id < m.slots.len() {
                            let slot = &m.slots[slot_id];
                            let show_overlay = slot.overlay_mode && !slot.last_translation.is_empty();
                            let show_border  = slot.show_frame;
                            let text = slot.last_translation.clone();
                            drop(m);

                            // Overlay mode: solid dark background (NOT pure black — black is our
                            // Win32 color key and would become transparent). Use dark navy instead.
                            if show_overlay {
                                painter.rect_filled(full_rect, 0.0, egui::Color32::from_rgb(15, 15, 30));

                                let galley = ctx.fonts(|f| {
                                    f.layout(
                                        text,
                                        egui::FontId::proportional(18.0),
                                        egui::Color32::WHITE,
                                        full_rect.width() - 16.0,
                                    )
                                });
                                let text_pos = egui::pos2(
                                    full_rect.center().x - galley.size().x / 2.0,
                                    full_rect.center().y - galley.size().y / 2.0,
                                );
                                painter.galley(text_pos, galley, egui::Color32::WHITE);
                            }

                            // Always draw the green frame border when show_frame is on
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
                    // ─────────────────────────────
                    // The glow backend hard-codes [0,0,0,0] clear for child viewports.
                    // That renders as solid RGB(0,0,0) = BLACK on-screen (no DWM alpha
                    // compositing for plain GL surfaces).  We tell DWM:
                    //   "any pixel that is exactly (0,0,0) should be transparent."
                    // WS_EX_LAYERED is already set by winit via .with_transparent(true).
                    #[cfg(target_os = "windows")]
                    unsafe {
                        use std::ptr;
                        use windows::Win32::Foundation::COLORREF;
                        use windows::Win32::UI::WindowsAndMessaging::{
                            FindWindowW, SetLayeredWindowAttributes, LWA_COLORKEY,
                        };
                        let title_w: Vec<u16> = format!("Frame Overlay {}\0", slot_id + 1)
                            .encode_utf16()
                            .collect();
                        let hwnd = FindWindowW(
                            windows::core::PCWSTR(ptr::null()),
                            windows::core::PCWSTR(title_w.as_ptr()),
                        );
                        if let Ok(hwnd) = hwnd {
                            if !hwnd.0.is_null() {
                                let _ = SetLayeredWindowAttributes(
                                    hwnd,
                                    COLORREF(0x000000), // pure black → transparent
                                    0,
                                    LWA_COLORKEY,
                                );
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

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn tick_background(&mut self) {
        // 1. Drain results from background threads
        while let Ok(result) = self.bg_rx.try_recv() {
            match result {
                BgResult::Done {
                    slot_idx,
                    ocr_text,
                    translated,
                    frame_hash,
                } => {
                    self.slot_busy[slot_idx] = false;
                    self.slot_processing[slot_idx] = false;
                    self.last_frame_hash[slot_idx] = frame_hash;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    let slot = &mut model.slots[slot_idx];
                    slot.next_tick_at_ms = now.saturating_add(slot.refresh_ms.max(500));

                    // Only update UI if the OCR text actually changed
                    // (prevents flickering when content is the same)
                    let new_ocr = ocr_text.trim();
                    let old_ocr = slot.last_ocr_text.trim();
                    let content_changed = !new_ocr.is_empty() && new_ocr != old_ocr;

                    if content_changed {
                        slot.last_ocr_text = ocr_text;
                        if !translated.trim().is_empty() {
                            slot.last_translation = translated;
                            slot.pending_text.clear();
                        }
                    }
                    // Clear any previous error on success
                    self.last_error = None;
                }
                BgResult::Unchanged { slot_idx } => {
                    self.slot_busy[slot_idx] = false;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    model.slots[slot_idx].next_tick_at_ms = now.saturating_add(model.slots[slot_idx].refresh_ms.max(200));
                }
                BgResult::HashChanged { slot_idx, new_hash } => {
                    self.slot_busy[slot_idx] = false;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    let slot = &mut model.slots[slot_idx];
                    slot.stable_hash = new_hash;
                    slot.stable_since_ms = now;
                    // Check very aggressively until stable (100ms)
                    slot.next_tick_at_ms = now.saturating_add(100);
                }
                BgResult::WaitingDebounce { slot_idx } => {
                    self.slot_busy[slot_idx] = false;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    // Keep aggressively checking until debounce passes
                    model.slots[slot_idx].next_tick_at_ms = now.saturating_add(100);
                }
                BgResult::CacheHit { slot_idx, ocr_text, translated, frame_hash } => {
                    self.slot_busy[slot_idx] = false;
                    self.slot_processing[slot_idx] = false;
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
                }
                BgResult::Error { slot_idx, err } => {
                    self.slot_busy[slot_idx] = false;
                    self.slot_processing[slot_idx] = false;
                    let now = Self::now_ms();
                    let mut model = self.model.lock();
                    model.slots[slot_idx].next_tick_at_ms = now.saturating_add(10_000);
                    self.last_error = Some(format!("Region {}: {err}", slot_idx + 1));
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
            let gemini = self.gemini_translator.clone();
            let tx = self.bg_tx.clone();
            let prev_hash = self.last_frame_hash[i];
            
            // Debounce variables
            let stable_hash = slot.stable_hash;
            let stable_since_ms = slot.stable_since_ms;
            let debounce_dur_ms = 800; // minimum time screen must be stable before OCR

            let cache_arc = self.translation_cache.clone();

            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<BgResult> {
                    let mut frame = capture.capture_rect(rect, display_id)?;

                    // Apply Black & White Thresholding directly to the image buffer
                    // This creates infinite contrast for local OCR.
                    for chunk in frame.data.chunks_exact_mut(4) {
                        let r = chunk[0] as f32;
                        let g = chunk[1] as f32;
                        let b = chunk[2] as f32;
                        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
                        let val = if lum > 128.0 { 255 } else { 0 };
                        chunk[0] = val;
                        chunk[1] = val;
                        chunk[2] = val;
                        chunk[3] = 255;
                    }

                    let hash = smart_hash(&frame.data);

                    // 1. Debounce state machine (Visual stability)
                    if hash != stable_hash {
                        return Ok(BgResult::HashChanged { slot_idx: i, new_hash: hash });
                    }
                    
                    let now = Self::now_ms();
                    if now.saturating_sub(stable_since_ms) < debounce_dur_ms {
                        return Ok(BgResult::WaitingDebounce { slot_idx: i });
                    }

                    // 2. Hash is fully stable! Has it changed since the LAST successful translation?
                    if hash == prev_hash && prev_hash != 0 {
                        return Ok(BgResult::Unchanged { slot_idx: i });
                    }

                    // 3. New stable scene. Check Cache!
                    {
                        let cache = cache_arc.lock();
                        if let Some((ocr, tra)) = cache.get(&hash) {
                            return Ok(BgResult::CacheHit {
                                slot_idx: i,
                                ocr_text: ocr.clone(),
                                translated: tra.clone(),
                                frame_hash: hash,
                            });
                        }
                    }
                    
                    // Signal heavy work starting (triggers UI spinner)
                    let _ = tx.send(BgResult::Translating { slot_idx: i });

                    // 4. Local OCR (Offline)
                    let ocr_text = windows_ocr.recognize(&frame, source_lang.as_ref())?;
                    let ocr_text = ocr_text.trim().to_string();

                    if ocr_text.is_empty() {
                        return Ok(BgResult::Done {
                            slot_idx: i,
                            ocr_text: String::new(),
                            translated: String::new(),
                            frame_hash: hash,
                        });
                    }

                    // 5. Hit Gemini API for TEXT ONLY translation.
                    if let Some(g) = &gemini {
                        let translated = g.translate(
                            &ocr_text,
                            source_lang.as_ref(),
                            &target_lang,
                        )?;
                        
                        // Save into cache
                        {
                            let mut cache = cache_arc.lock();
                            cache.insert(hash, (ocr_text.clone(), translated.clone()));
                        }

                        Ok(BgResult::Done {
                            slot_idx: i,
                            ocr_text,
                            translated,
                            frame_hash: hash,
                        })
                    } else {
                        anyhow::bail!("Gemini API key not set — click ⚙ to configure");
                    }
                })();

                match result {
                    Ok(bg_res) => {
                        let _ = tx.send(bg_res);
                    }
                    Err(e) => {
                        let _ = tx.send(BgResult::Error { slot_idx: i, err: format!("{e:#}") });
                    }
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
                    self.last_error = None;
                }
                Ok(_) => {
                    self.last_error =
                        Some("listModels returned empty (using built-in list)".to_string());
                }
                Err(e) => {
                    self.last_error = Some(format!(
                        "{e:#}\n(using built-in model list; check API key or click Refresh)"
                    ));
                }
            }
        }
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Gemini (API key)");
                ui.horizontal(|ui| {
                    ui.label("model");
                    let choices = self.model_choices();
                    let mut current_idx = choices
                        .iter()
                        .position(|m| m.id == self.settings.gemini_model)
                        .unwrap_or(0);
                    egui::ComboBox::from_id_salt("gemini_model_dropdown")
                        .width(280.0)
                        .selected_text(
                            choices
                                .get(current_idx)
                                .map(|m| m.display_name.as_str())
                                .unwrap_or(self.settings.gemini_model.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for (i, m) in choices.iter().enumerate() {
                                ui.selectable_value(&mut current_idx, i, &m.display_name);
                            }
                        });
                    if let Some(sel) = choices.get(current_idx) {
                        self.settings.gemini_model = sel.id.clone();
                    }
                    if ui.button("Refresh").clicked() {
                        match GeminiTranslator::list_models(&self.settings.gemini_api_key) {
                            Ok(models) => {
                                self.gemini_models = models;
                                self.last_error = None;
                            }
                            Err(e) => self.last_error = Some(format!("{e:#}")),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("");
                    ui.small(
                        "Pick from list (built-in until Refresh loads your account's models).",
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("api_key");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.gemini_api_key)
                            .password(true),
                    );
                });
                ui.separator();
                if ui.button("Save").clicked() {
                    if let Err(e) = save_settings(&self.settings) {
                        self.last_error = Some(format!("{e:#}"));
                    } else {
                        // Recreate the translator adapter with new key/model
                        self.gemini_translator = GeminiTranslator::new(
                            self.settings.gemini_api_key.clone(),
                            self.settings.gemini_model.clone(),
                        )
                        .ok()
                        .map(Arc::new);
                        self.last_error = None;
                    }
                }
            });
        self.show_settings = open;
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Essential for transparent viewports: 
        // Force the GPU background clear color to be fully transparent.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_background();

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
                    }
                    
                    ui.add_space(8.0);
                    
                    if let Some(err) = &self.last_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("⚠️ {err}"));
                    }

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
                    self.last_frame_hash.push(0);
                }
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
                self.last_frame_hash.remove(idx);
                
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
