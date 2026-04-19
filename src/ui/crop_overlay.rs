use std::sync::Arc;

use egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use parking_lot::Mutex;
use screenshots::Screen;

use crate::core::types::Rect;

/// Result of the region-crop overlay (one shot).
pub enum CropOutcome {
    Cancelled,
    Done { slot: usize, rect: Rect },
}

pub struct CropOverlayState {
    pub slot_idx: usize,
    pub texture: egui::TextureHandle,
    /// Top-left of this capture in **screen** coordinates (multi-monitor).
    pub origin: (i32, i32),
    pub px: (u32, u32),
    drag_start: Option<Pos2>,
    drag_current: Option<Pos2>,
}

impl CropOverlayState {
    /// Capture the **primary** monitor (fallback: first monitor) and build a texture.
    pub fn start(slot_idx: usize, display_id: u32, ctx: &egui::Context) -> anyhow::Result<Self> {
        let screens = Screen::all()?;
        let screen = screens
            .iter()
            .find(|s| s.display_info.id == display_id)
            .or_else(|| screens.iter().find(|s| s.display_info.is_primary))
            .or_else(|| screens.first())
            .ok_or_else(|| anyhow::anyhow!("no display found"))?;
        let img = screen.capture()?;
        let w = img.width();
        let h = img.height();
        let rgba = img.into_raw();
        let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        let tid = format!(
            "crop_{}_{}",
            slot_idx,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let texture = ctx.load_texture(tid, color, Default::default());
        Ok(Self {
            slot_idx,
            texture,
            origin: (screen.display_info.x, screen.display_info.y),
            px: (w, h),
            drag_start: None,
            drag_current: None,
        })
    }

    fn pixel_to_screen(&self, img_rect: &egui::Rect, p: Pos2) -> (i32, i32) {
        let w = self.px.0 as f32;
        let h = self.px.1 as f32;
        let nx = ((p.x - img_rect.min.x) / img_rect.width()).clamp(0.0, 1.0);
        let ny = ((p.y - img_rect.min.y) / img_rect.height()).clamp(0.0, 1.0);
        let px = (nx * w) as i32;
        let py = (ny * h) as i32;
        (self.origin.0 + px, self.origin.1 + py)
    }

    fn try_finish_rect(&self, img_rect: &egui::Rect, a: Pos2, b: Pos2) -> Option<Rect> {
        let (sx1, sy1) = self.pixel_to_screen(img_rect, a);
        let (sx2, sy2) = self.pixel_to_screen(img_rect, b);
        let x = sx1.min(sx2);
        let y = sy1.min(sy2);
        let w = (sx1 - sx2).abs();
        let h = (sy1 - sy2).abs();
        if w < 8 || h < 8 {
            return None;
        }
        Some(Rect { x, y, w, h })
    }
}

/// Run the fullscreen crop viewport; call **every frame** while `state` is active.
pub fn run_crop_viewport(
    ctx: &egui::Context,
    state: Arc<Mutex<CropOverlayState>>,
    outcome: Arc<Mutex<Option<CropOutcome>>>,
) {
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("screen_translator_region_crop"),
        egui::ViewportBuilder::default()
            .with_title("Drag to select region — release to confirm, Esc to cancel")
            .with_fullscreen(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop),
        |ctx, class| {
            if matches!(class, egui::ViewportClass::Embedded) {
                egui::Window::new("Crop region")
                    .collapsible(false)
                    .resizable(true)
                    .show(ctx, |ui| {
                        crop_content(ui, &state, &outcome);
                    });
            } else {
                egui::CentralPanel::default().show(ctx, |ui| {
                    crop_content(ui, &state, &outcome);
                });
            }
        },
    );
}

fn crop_content(
    ui: &mut egui::Ui,
    state: &Arc<Mutex<CropOverlayState>>,
    outcome: &Arc<Mutex<Option<CropOutcome>>>,
) {
    if outcome.lock().is_some() {
        return;
    }

    let mut st = state.lock();

    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        drop(st);
        *outcome.lock() = Some(CropOutcome::Cancelled);
        return;
    }

    let tex = st.texture.clone();
    let (iw, ih) = (st.px.0 as f32, st.px.1 as f32);

    ui.vertical(|ui| {
        ui.label(egui::RichText::new("Click and drag to draw a region (select text area to capture)").strong());
        ui.small("Using primary monitor — multi-monitor support can be added later.");
    });

    let available = ui.available_size();
    let scale = (available.x / iw).min(available.y / ih);
    let size = Vec2::new(iw * scale, ih * scale);

    let response = ui.add(
        egui::Image::from_texture(&tex)
            .fit_to_exact_size(size)
            .sense(Sense::click_and_drag()),
    );

    let img_rect = response.rect;

    if response.drag_started() {
        if let Some(p) = response.interact_pointer_pos() {
            st.drag_start = Some(p);
            st.drag_current = Some(p);
        }
    }
    if response.dragged() {
        if let Some(p) = response.interact_pointer_pos() {
            st.drag_current = Some(p);
        }
    }
    if response.drag_stopped() {
        if let (Some(a), Some(b)) = (st.drag_start, st.drag_current) {
            if let Some(rect) = st.try_finish_rect(&img_rect, a, b) {
                let slot = st.slot_idx;
                drop(st);
                *outcome.lock() = Some(CropOutcome::Done { slot, rect });
                return;
            }
        }
        st.drag_start = None;
        st.drag_current = None;
    }

    if let (Some(a), Some(b)) = (st.drag_start, st.drag_current) {
        let r = egui::Rect::from_two_pos(a, b);
        ui.painter().rect_stroke(
            r,
            0.0,
            Stroke::new(2.0, Color32::from_rgb(0, 255, 128)),
            egui::StrokeKind::Outside,
        );
    }
}
