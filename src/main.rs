#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod camera;

use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("カメラビューア")
        .with_inner_size([1280.0, 720.0])
        .with_decorations(true);

    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "cam_viewer",
        options,
        Box::new(|cc| Ok(Box::new(app::CamViewerApp::new(cc)))),
    )
}

/// ウィンドウのタイトルバー／タスクバー表示用アイコンを読み込む。
/// 先に EXE と同じディレクトリの `icon.png`、無ければプロジェクトの
/// `assets/icon.png` を探す。どちらも無ければアイコン無しで起動する。
fn load_window_icon() -> Option<egui::IconData> {
    let candidates: [Option<PathBuf>; 2] = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("icon.png"))),
        Some(PathBuf::from("assets/icon.png")),
    ];
    for path in candidates.into_iter().flatten() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            continue;
        };
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        return Some(egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        });
    }
    None
}
