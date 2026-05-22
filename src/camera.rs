use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, RequestedFormat, RequestedFormatType,
};
use nokhwa::{query, Camera, NokhwaError};

/// UI スレッドから取得スレッドへ送る命令。
pub enum CamCommand {
    /// 指定デバイスを（必要なら指定フォーマットで）開く。`format` が `None` のときは最高解像度を自動選択。
    Open { index: u32, format: Option<CameraFormat> },
}

/// 1 フレーム分の RGB8 画像。
pub struct FrameMsg {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// 取得スレッドから UI スレッドへ返すイベント。
pub enum CamEvent {
    Frame(FrameMsg),
    /// カメラを開いた直後の、対応フォーマット一覧と現在のフォーマット。
    Formats {
        index: u32,
        formats: Vec<CameraFormat>,
        current: CameraFormat,
    },
    Error(String),
}

/// 接続されているカメラの一覧を `(index, 表示名)` で返す。UI スレッドから呼ぶ。
pub fn list_devices() -> Vec<(u32, String)> {
    query(ApiBackend::Auto)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, info)| {
            let idx = match info.index() {
                CameraIndex::Index(n) => *n,
                CameraIndex::String(_) => i as u32,
            };
            (idx, info.human_name())
        })
        .collect()
}

/// 取得スレッドを起動し、命令送信側とイベント受信側を返す。
pub fn spawn_capture() -> (Sender<CamCommand>, Receiver<CamEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<CamCommand>();
    let (evt_tx, evt_rx) = mpsc::channel::<CamEvent>();
    thread::spawn(move || capture_loop(cmd_rx, evt_tx));
    (cmd_tx, evt_rx)
}

fn capture_loop(cmd_rx: Receiver<CamCommand>, evt_tx: Sender<CamEvent>) {
    // Camera は COM/V4L のハンドルを握るためこのスレッド内だけで生成・保持する。
    let mut camera: Option<Camera> = None;

    loop {
        // 溜まっている命令をすべて処理（解像度/カメラ切替に素早く反応するため）。
        loop {
            match cmd_rx.try_recv() {
                Ok(CamCommand::Open { index, format }) => {
                    camera = None; // 既存ストリームを先に閉じる
                    match open_camera(index, format) {
                        Ok((cam, formats, current)) => {
                            let _ = evt_tx.send(CamEvent::Formats { index, formats, current });
                            camera = Some(cam);
                        }
                        Err(e) => {
                            let _ = evt_tx.send(CamEvent::Error(e.to_string()));
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // フレーム取得。
        if let Some(cam) = camera.as_mut() {
            match cam.frame().and_then(|b| b.decode_image::<RgbFormat>()) {
                Ok(img) => {
                    let (width, height) = (img.width(), img.height());
                    let _ = evt_tx.send(CamEvent::Frame(FrameMsg {
                        width,
                        height,
                        rgb: img.into_raw(),
                    }));
                }
                Err(e) => {
                    let _ = evt_tx.send(CamEvent::Error(e.to_string()));
                    thread::sleep(Duration::from_millis(100));
                }
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn open_camera(
    index: u32,
    format: Option<CameraFormat>,
) -> Result<(Camera, Vec<CameraFormat>, CameraFormat), NokhwaError> {
    let req_ty = match format {
        Some(f) => RequestedFormatType::Closest(f),
        None => RequestedFormatType::AbsoluteHighestResolution,
    };
    let req = RequestedFormat::new::<RgbFormat>(req_ty);
    let mut cam = Camera::new(CameraIndex::Index(index), req)?;

    // このデバイスが実際に出力できる組み合わせだけを列挙し、解像度→FPS の降順に並べる。
    let mut formats = cam.compatible_camera_formats().unwrap_or_default();
    formats.sort_by(|a, b| {
        let area_a = a.resolution().width() as u64 * a.resolution().height() as u64;
        let area_b = b.resolution().width() as u64 * b.resolution().height() as u64;
        (area_b, b.frame_rate()).cmp(&(area_a, a.frame_rate()))
    });

    cam.open_stream()?;
    let current = cam.camera_format();
    Ok((cam, formats, current))
}
