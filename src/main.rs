mod core;
mod adapters;
mod infra;
mod ui;

fn main() -> eframe::Result<()> {
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
