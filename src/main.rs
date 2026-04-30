#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod core;
mod adapters;
mod infra;
mod ui;

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::HiDpi::*;
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "KTranslator",
        native_options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    )
}
