use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
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
    /// 取得スレッドが直近 1 秒間に受け取ったフレーム数から算出した実測 FPS。
    MeasuredFps(f32),
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
    let mut frame_count: u32 = 0;
    let mut last_fps_report = Instant::now();

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
                            // FPS 計測カウンタをリセット
                            frame_count = 0;
                            last_fps_report = Instant::now();
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
                    frame_count += 1;
                    let elapsed = last_fps_report.elapsed();
                    if elapsed >= Duration::from_secs(1) {
                        let fps = frame_count as f32 / elapsed.as_secs_f32();
                        let _ = evt_tx.send(CamEvent::MeasuredFps(fps));
                        frame_count = 0;
                        last_fps_report = Instant::now();
                    }
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

    let raw = cam.compatible_camera_formats().unwrap_or_default();
    let formats = dedup_and_sort_formats(raw);

    cam.open_stream()?;
    let current = cam.camera_format();
    Ok((cam, formats, current))
}

/// 同じ (解像度, FPS) でピクセル形式違いの組合せが多数並ぶと選びにくいので、
/// 1 組につき 1 つだけ残す。優先順位はデコード相性と高 FPS 実現性で決める。
/// また、ドライバが申告する 1〜数 fps のような実用にならない低 FPS は除外する。
fn dedup_and_sort_formats(formats: Vec<CameraFormat>) -> Vec<CameraFormat> {
    use std::collections::HashMap;
    const MIN_FPS: u32 = 5;
    let mut best: HashMap<(u32, u32, u32), CameraFormat> = HashMap::new();
    for f in formats {
        if f.frame_rate() < MIN_FPS {
            continue;
        }
        let key = (
            f.resolution().width(),
            f.resolution().height(),
            f.frame_rate(),
        );
        let take = match best.get(&key) {
            Some(cur) => rank_pixel_format(f.format()) < rank_pixel_format(cur.format()),
            None => true,
        };
        if take {
            best.insert(key, f);
        }
    }
    let mut out: Vec<CameraFormat> = best.into_values().collect();
    out.sort_by(|a, b| {
        let area_a = a.resolution().width() as u64 * a.resolution().height() as u64;
        let area_b = b.resolution().width() as u64 * b.resolution().height() as u64;
        (area_b, b.frame_rate()).cmp(&(area_a, a.frame_rate()))
    });
    out
}

/// 小さいほど優先。MJPEG は USB カメラで高解像度・高 FPS をサポートしやすいので最優先。
fn rank_pixel_format(f: FrameFormat) -> u8 {
    match f {
        FrameFormat::MJPEG => 0,
        FrameFormat::YUYV => 1,
        FrameFormat::NV12 => 2,
        FrameFormat::GRAY => 3,
        FrameFormat::RAWRGB => 4,
        FrameFormat::RAWBGR => 5,
    }
}
