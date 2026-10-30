//! clothing-maker: editor de patrones con vista 3D.
//!
//! Uso: `clothing-maker [fichero.pattern.json]`

mod app;
mod canvas;
mod examples;
mod gpu;
mod history;
mod icons;
mod section;
mod stages;
mod theme;
mod view3d;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    match run(eframe::Renderer::Wgpu, file.clone()) {
        // Si wgpu no arranca de ninguna forma, al menos se abre el editor 2D
        // con OpenGL (la vista 3D lo avisa).
        Err(eframe::Error::Wgpu(e)) => {
            eprintln!("No se ha podido usar wgpu ({e}); se prueba con OpenGL, sin vista 3D.");
            run(eframe::Renderer::Glow, file)
        }
        result => result,
    }
}

fn run(renderer: eframe::Renderer, file: Option<std::path::PathBuf>) -> eframe::Result {
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("clothing-maker")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        renderer,
        ..Default::default()
    };
    options.wgpu_options.wgpu_setup = gpu::setup();
    eframe::run_native("clothing-maker", options, Box::new(|cc| Ok(Box::new(app::ClothingMakerApp::new(cc, file)))))
}
