use eframe::egui;
use crate::core::{
    model::AppModel,
    types::{LanguageTag, Rect},
};
use crate::core::worker::SlotRuntimeState;

pub struct SlotUiResponse {
    pub do_crop: bool,
    pub should_remove: bool,
}

pub const LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("Auto Detection",                ""),
    ("Thai (th)",                     "th"),
    ("English (en)",                  "en"),
    ("Japanese (ja)",                 "ja"),
    ("Korean (ko)",                   "ko"),
    ("Chinese Simplified (zh-Hans)",  "zh-Hans"),
    ("Chinese Traditional (zh-Hant)", "zh-Hant"),
    ("French (fr)",                   "fr"),
    ("German (de)",                   "de"),
    ("Spanish (es)",                  "es"),
    ("Italian (it)",                  "it"),
    ("Portuguese (pt)",               "pt"),
    ("Russian (ru)",                  "ru"),
    ("Ukrainian (uk)",                "uk"),
    ("Bulgarian (bg)",                "bg"),
    ("Serbian (sr)",                  "sr"),
    ("Croatian (hr)",                 "hr"),
    ("Swahili (sw)",                  "sw"),
    ("Afrikaans (af)",                "af"),
];

pub fn render_slot_item(
    ui: &mut egui::Ui,
    slot_idx: usize,
    model: &mut AppModel,
    runtime: &SlotRuntimeState,
    available_screens: &[(u32, String)],
) -> SlotUiResponse {
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
            let slot = &mut model.slots[slot_idx];

            egui::ComboBox::from_id_salt(format!("disp_sel_{}", slot_idx))
                .selected_text({
                    available_screens.iter()
                        .find(|(id, _)| *id == slot.display_id)
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "Primary".to_string())
                })
                .show_ui(ui, |ui| {
                    for (id, name) in available_screens {
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
            let slot = &mut model.slots[slot_idx];

            ui.label("🌐 From:");
            let mut src = slot.source_lang.as_ref().map(|l| l.0.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt(format!("src_{slot_idx}"))
                .selected_text(
                    LANGUAGE_OPTIONS.iter()
                        .find(|(_, code)| code.to_string() == src)
                        .map(|(name, _)| *name).unwrap_or("Auto Detection"),
                )
                .show_ui(ui, |ui| {
                    for (name, code) in LANGUAGE_OPTIONS {
                        ui.selectable_value(&mut src, code.to_string(), *name);
                    }
                });
            slot.source_lang = if src.is_empty() { None } else { Some(LanguageTag(src)) };

            ui.add_space(10.0);
            ui.label("➡️ To:");
            let mut tgt = slot.target_lang.0.clone();
            egui::ComboBox::from_id_salt(format!("tgt_{slot_idx}"))
                .selected_text(
                    LANGUAGE_OPTIONS.iter()
                        .find(|(_, code)| code.to_string() == tgt)
                        .map(|(name, _)| *name).unwrap_or("Thai (th)"),
                )
                .show_ui(ui, |ui| {
                    for (name, code) in LANGUAGE_OPTIONS {
                        if code.is_empty() { continue; }
                        ui.selectable_value(&mut tgt, code.to_string(), *name);
                    }
                });
            slot.target_lang = LanguageTag(tgt);
        });

        ui.add_space(8.0);

        // --- VIEW OPTIONS ROW ---
        ui.horizontal(|ui| {
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

        ui.horizontal(|ui| {
            if runtime.processing || runtime.busy {
                ui.add(egui::Spinner::new().size(12.0));
            } else {
                ui.label("💤");
            }
            ui.label(egui::RichText::new(&runtime.status).size(13.0).strong());
        });
    });

    SlotUiResponse { do_crop, should_remove }
}
