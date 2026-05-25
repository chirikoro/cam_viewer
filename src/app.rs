use eframe::egui;
use nokhwa::utils::CameraFormat;

use crate::camera::{self, CamCommand, CamEvent, FrameMsg};

pub struct CamViewerApp {
    cmd_tx: std::sync::mpsc::Sender<CamCommand>,
    evt_rx: std::sync::mpsc::Receiver<CamEvent>,
    texture: Option<egui::TextureHandle>,
    img_size: [usize; 2],
    devices: Vec<(u32, String)>,
    selected_device: u32,
    formats: Vec<CameraFormat>,
    selected_format: Option<CameraFormat>,
    // 直前にユーザーが要求したフォーマット。Closest で別物に置き換わった時に
    // 「要求 vs 実際」の差分を提示するために使う。
    last_requested: Option<CameraFormat>,
    status: Option<String>,
    settings_open: bool,
    fullscreen: bool,
    last_error: Option<String>,
}

impl CamViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_japanese_font(&cc.egui_ctx);

        let (cmd_tx, evt_rx) = camera::spawn_capture();
        let devices = camera::list_devices();
        let selected_device = devices.first().map(|d| d.0).unwrap_or(0);

        // 起動時は最高解像度・FPS を自動選択して開く。
        let _ = cmd_tx.send(CamCommand::Open {
            index: selected_device,
            format: None,
        });

        Self {
            cmd_tx,
            evt_rx,
            texture: None,
            img_size: [0, 0],
            devices,
            selected_device,
            formats: Vec::new(),
            selected_format: None,
            last_requested: None,
            status: Some("カメラを開いています…".to_owned()),
            settings_open: false,
            fullscreen: false,
            last_error: None,
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("設定")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                // カメラ選択
                let devices = self.devices.clone();
                let cur_name = devices
                    .iter()
                    .find(|d| d.0 == self.selected_device)
                    .map(|d| d.1.clone())
                    .unwrap_or_else(|| "(なし)".to_owned());
                egui::ComboBox::from_label("カメラ")
                    .selected_text(cur_name)
                    .show_ui(ui, |ui| {
                        for (idx, name) in &devices {
                            if ui
                                .selectable_label(*idx == self.selected_device, name)
                                .clicked()
                                && *idx != self.selected_device
                            {
                                self.selected_device = *idx;
                                self.selected_format = None;
                                self.formats.clear();
                                self.last_requested = None;
                                self.status = Some(format!("『{}』を開いています…", name));
                                let _ = self.cmd_tx.send(CamCommand::Open {
                                    index: *idx,
                                    format: None,
                                });
                            }
                        }
                    });

                // 解像度・FPS 選択（接続カメラが対応する組み合わせのみ。
                // 同じ解像度・FPS でピクセル形式違いの重複は内部で集約済み）
                let formats = self.formats.clone();
                let fmt_label = self
                    .selected_format
                    .map(fmt_to_string)
                    .unwrap_or_else(|| "(自動)".to_owned());
                egui::ComboBox::from_label("解像度・FPS")
                    .selected_text(fmt_label)
                    .show_ui(ui, |ui| {
                        for f in &formats {
                            let selected = self.selected_format == Some(*f);
                            if ui.selectable_label(selected, fmt_to_string(*f)).clicked() {
                                self.last_requested = Some(*f);
                                self.status =
                                    Some(format!("要求中: {} …", fmt_to_string(*f)));
                                let _ = self.cmd_tx.send(CamCommand::Open {
                                    index: self.selected_device,
                                    format: Some(*f),
                                });
                            }
                        }
                    });

                // 現在カメラに実際に適用されているフォーマット（ピクセル形式まで含めて表示）
                if let Some(cur) = self.selected_format {
                    ui.label(format!(
                        "現在のフォーマット: {} ({:?})",
                        fmt_to_string(cur),
                        cur.format()
                    ));
                }

                // 全画面表示
                let mut fs = self.fullscreen;
                if ui.checkbox(&mut fs, "全画面表示").changed() {
                    self.set_fullscreen(ctx, fs);
                }

                ui.separator();
                if let Some(s) = &self.status {
                    ui.label(s);
                }
                if let Some(err) = &self.last_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("エラー: {err}"));
                }
                ui.label("右クリックで設定の開閉 / F11 で全画面");
            });
        self.settings_open = open;
    }
}

impl eframe::App for CamViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 溜まったイベントを処理し、フレームは最新の 1 枚だけ採用する（遅延防止）。
        let mut latest: Option<FrameMsg> = None;
        while let Ok(evt) = self.evt_rx.try_recv() {
            match evt {
                CamEvent::Frame(f) => latest = Some(f),
                CamEvent::Formats {
                    index,
                    formats,
                    current,
                } => {
                    if index == self.selected_device {
                        self.formats = formats;
                        self.selected_format = Some(current);
                        self.last_error = None;
                        let req = self.last_requested.take();
                        self.status = Some(match req {
                            Some(r) if r == current => {
                                format!("適用しました: {}", fmt_to_string(current))
                            }
                            Some(r) => format!(
                                "『{}』はこのカメラで使えないため『{}』を適用しました",
                                fmt_to_string(r),
                                fmt_to_string(current)
                            ),
                            None => format!("適用中: {}", fmt_to_string(current)),
                        });
                    }
                }
                CamEvent::Error(e) => self.last_error = Some(e),
            }
        }

        if let Some(f) = latest {
            let image =
                egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
            self.img_size = [f.width as usize, f.height as usize];
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture =
                        Some(ctx.load_texture("camera_frame", image, egui::TextureOptions::LINEAR))
                }
            }
        }

        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            let next = !self.fullscreen;
            self.set_fullscreen(ctx, next);
        }

        let frame = egui::Frame::default().fill(egui::Color32::BLACK);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.max_rect();

            if let (Some(tex), [w, h]) = (&self.texture, self.img_size) {
                if w > 0 && h > 0 {
                    let draw = fit_contain(rect, w as f32, h as f32);
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(tex.id(), draw, uv, egui::Color32::WHITE);
                }
            }

            // 映像領域の右クリックで設定を開閉。
            let resp = ui.interact(rect, ui.id().with("video_area"), egui::Sense::click());
            if resp.secondary_clicked() {
                self.settings_open = !self.settings_open;
            }
        });

        if self.settings_open {
            self.show_settings(ctx);
        }

        // 映像更新のため継続的に再描画。
        ctx.request_repaint();
    }
}

/// アスペクト比を保ったまま `container` に収める矩形（contain）。縦も横もはみ出さず、
/// 余白は背景の黒がそのまま黒帯になる。
fn fit_contain(container: egui::Rect, img_w: f32, img_h: f32) -> egui::Rect {
    let scale = (container.width() / img_w).min(container.height() / img_h);
    egui::Rect::from_center_size(
        container.center(),
        egui::vec2(img_w * scale, img_h * scale),
    )
}

fn fmt_to_string(f: CameraFormat) -> String {
    format!(
        "{}x{} @ {}fps",
        f.resolution().width(),
        f.resolution().height(),
        f.frame_rate()
    )
}

/// Windows のシステムフォントを読み込み、日本語が表示できるようにする。
fn install_japanese_font(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\YuGothM.ttc",  // 游ゴシック Medium
        r"C:\Windows\Fonts\meiryo.ttc",   // メイリオ
        r"C:\Windows\Fonts\msgothic.ttc", // MS ゴシック
    ];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
        // どれも読めなければ既定フォント（英数字のみ）のまま動作させる。
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    // .ttc コレクションは先頭フォント（index 0）を使用する。
    fonts
        .font_data
        .insert("jp".to_owned(), egui::FontData::from_owned(bytes).into());
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().insert(0, "jp".to_owned());
    }
    ctx.set_fonts(fonts);
}
