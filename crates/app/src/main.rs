//! clothing-maker: editor de patrones.
//!
//! Uso: `clothing-maker [fichero.pattern.json]`

mod app;
mod canvas;
mod examples;
mod history;

use eframe::egui;

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("clothing-maker")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("clothing-maker", options, Box::new(|cc| Ok(Box::new(app::ClothingMakerApp::new(cc, file)))))
}
