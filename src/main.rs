#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod camera;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("カメラビューア")
            .with_inner_size([1280.0, 720.0])
            .with_decorations(true),
        ..Default::default()
    };

    eframe::run_native(
        "cam_viewer",
        options,
        Box::new(|cc| Ok(Box::new(app::CamViewerApp::new(cc)))),
    )
}
