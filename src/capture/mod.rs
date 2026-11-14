//! D2R capture thread: window tracking, WGC/DXGI capture, downscale, and the
//! load-screen state machine.

pub mod win32;

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Graphics::Capture::Direct3D11CaptureFrame;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::config::CaptureConfig;
use crate::detector;
use crate::state::{CaptureStatus, SharedState};
use win32::dxgi::DxgiCapture;
use win32::scale::{Downscaler, ScaledFrame};
use win32::wgc::WgcCapture;
use win32::window;

/// The detector is tuned for 60 captured frames per second. Capture FPS is not
/// configurable.
const CAPTURE_FPS: u32 = 60;
/// Slightly below 16.67 ms so a regular 60 Hz source is never mistaken for a
/// duplicate frame.
const CAPTURE_INTERVAL: Duration = Duration::from_millis(16);
/// If no new frame arrives for this long, assume capture is stuck. WGC restarts;
/// DXGI only resumes game time, because a static desktop can legitimately stop
/// producing duplication frames.
const NO_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// Window visibility and client-size checks run this often even while frames
/// keep arriving. WGC can keep presenting the last frame of a minimized window,
/// which would otherwise keep a loading screen paused indefinitely.
const WINDOW_CHECK_INTERVAL: Duration = Duration::from_secs(2);

pub struct CapturedTexture {
    pub texture: ID3D11Texture2D,
    pub timestamp: i64,
    pub size: (u32, u32),
    /// WGC frame objects must stay alive until the texture has been copied.
    pub _keepalive: Option<Direct3D11CaptureFrame>,
}

#[derive(Debug)]
pub struct CaptureError(String);

impl CaptureError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Wgc,
    Dxgi,
}

enum ActiveCapture {
    Wgc(WgcCapture),
    Dxgi(DxgiCapture),
}

impl ActiveCapture {
    fn kind(&self) -> Backend {
        match self {
            Self::Wgc(_) => Backend::Wgc,
            Self::Dxgi(_) => Backend::Dxgi,
        }
    }

    fn size(&self) -> Option<(u32, u32)> {
        match self {
            Self::Wgc(capture) => Some(capture.size()),
            Self::Dxgi(_) => None,
        }
    }

    fn matches_monitor(&self, hwnd: windows::Win32::Foundation::HWND) -> bool {
        match self {
            Self::Wgc(_) => true,
            Self::Dxgi(capture) => capture.matches_monitor(hwnd),
        }
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<CapturedTexture>, CaptureError> {
        match self {
            Self::Wgc(capture) => capture.next_frame(timeout),
            Self::Dxgi(capture) => capture.next_frame(timeout),
        }
    }
}

enum BreakReason {
    Lost,
    Restart,
    Shutdown,
    BlackFallback,
}

struct Runtime {
    confirm: ConfirmState,
    loading: bool,
    had_capture: bool,
    black_since: Option<Instant>,
}

struct ConfirmState {
    stable: bool,
    candidate: Option<(bool, u32)>,
}

impl ConfirmState {
    fn new() -> Self {
        Self {
            stable: false,
            candidate: None,
        }
    }

    fn reset(&mut self) {
        self.stable = false;
        self.candidate = None;
    }

    /// Returns the confirmed state after `required` consecutive frames.
    fn update(&mut self, value: bool, required: u32) -> bool {
        if value == self.stable {
            self.candidate = None;
            return self.stable;
        }

        let required = required.max(1);
        match self.candidate {
            Some((candidate, count)) if candidate == value => {
                let count = count + 1;
                if count >= required {
                    self.stable = value;
                    self.candidate = None;
                } else {
                    self.candidate = Some((candidate, count));
                }
            }
            _ => {
                if required <= 1 {
                    self.stable = value;
                    self.candidate = None;
                } else {
                    self.candidate = Some((value, 1));
                }
            }
        }

        self.stable
    }
}

pub fn run(shared: Arc<SharedState>, save_frames: Option<PathBuf>) {
    let mut dumper = match save_frames {
        Some(directory) => FrameDumper::new(directory).ok(),
        None => None,
    };
    let com_initialized = unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
        .is_ok()
    };

    let device = match create_device() {
        Ok(device) => device,
        Err(error) => {
            shared.set_capture_status(CaptureStatus::Error, error);
            shared.set_desired_paused(false);
            if com_initialized {
                unsafe { windows::Win32::System::WinRT::RoUninitialize() };
            }
            return;
        }
    };

    let mut scaler = match Downscaler::new(&device) {
        Ok(scaler) => scaler,
        Err(error) => {
            shared.set_capture_status(CaptureStatus::Error, error.to_string());
            shared.set_desired_paused(false);
            if com_initialized {
                unsafe { windows::Win32::System::WinRT::RoUninitialize() };
            }
            return;
        }
    };

    let mut runtime = Runtime {
        confirm: ConfirmState::new(),
        loading: false,
        had_capture: false,
        black_since: None,
    };
    let mut backend = Backend::Wgc;
    let mut wgc_failures = 0u32;
    let mut last_capture_config: Option<CaptureConfig> = None;

    'main: loop {
        let snapshot = shared.snapshot();
        if snapshot.shutdown {
            break;
        }

        let capture_config = snapshot.settings.capture.clone();
        if last_capture_config
            .as_ref()
            .map(|previous| previous != &capture_config)
            .unwrap_or(true)
        {
            last_capture_config = Some(capture_config.clone());
            backend = Backend::Wgc;
            wgc_failures = 0;
        }

        let Some(found) = window::find_d2r_window(&capture_config) else {
            runtime.confirm.reset();
            runtime.loading = false;
            runtime.black_since = None;

            if runtime.had_capture {
                shared.set_capture_status(CaptureStatus::Lost, "D2R window lost");
            } else {
                shared.set_capture_status(CaptureStatus::Searching, "waiting for D2R");
            }
            // Game time only pauses for loading screens. If D2R is closed or
            // capture is lost, the timer keeps running.
            shared.set_desired_paused(false);

            shared.wait_for_revision(snapshot.revision, Duration::from_millis(500));
            continue;
        };

        let expected_size = (found.client_width, found.client_height);
        let mut capture = match start_capture(backend, &device, found.hwnd) {
            Ok(capture) => capture,
            Err(error) => {
                if backend == Backend::Wgc {
                    wgc_failures += 1;
                    backend = Backend::Dxgi;
                    continue;
                }

                shared.set_capture_status(CaptureStatus::Error, error.to_string());
                shared.set_desired_paused(false);
                shared.wait_for_revision(snapshot.revision, Duration::from_secs(2));
                continue;
            }
        };

        runtime.had_capture = true;
        shared.set_capture_status(CaptureStatus::Capturing, capture_detail(expected_size));

        let confirm_frames = snapshot.settings.detection.confirm_frames.max(1);
        let session_started = Instant::now();
        let reason = capture_session(
            &shared,
            &capture_config,
            confirm_frames,
            found.hwnd,
            expected_size,
            &mut capture,
            &mut scaler,
            &mut runtime,
            dumper.as_mut(),
        );
        if session_started.elapsed() >= Duration::from_secs(5) {
            wgc_failures = 0;
        }

        match reason {
            BreakReason::Shutdown => break 'main,
            BreakReason::Restart => {
                runtime.black_since = None;
                continue;
            }
            BreakReason::BlackFallback => {
                runtime.black_since = None;
                if capture.kind() == Backend::Wgc {
                    wgc_failures += 1;
                    backend = Backend::Dxgi;
                }
                continue;
            }
            BreakReason::Lost => {
                runtime.confirm.reset();
                runtime.loading = false;
                runtime.black_since = None;
                if capture.kind() == Backend::Wgc {
                    wgc_failures += 1;
                    if wgc_failures >= 3 {
                        backend = Backend::Dxgi;
                    }
                }
                shared.set_capture_status(CaptureStatus::Lost, "capture lost");
                // A lost capture is not a loading screen; keep game time running.
                shared.set_desired_paused(false);
                shared.wait_for_revision(shared.snapshot().revision, Duration::from_millis(200));
            }
        }
    }

    shared.set_desired_paused(false);
    shared.set_capture_status(CaptureStatus::Off, "");
    if com_initialized {
        unsafe { windows::Win32::System::WinRT::RoUninitialize() };
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_session(
    shared: &SharedState,
    capture_config: &CaptureConfig,
    confirm_frames: u32,
    hwnd: windows::Win32::Foundation::HWND,
    expected_size: (u32, u32),
    capture: &mut ActiveCapture,
    scaler: &mut Downscaler,
    runtime: &mut Runtime,
    mut dumper: Option<&mut FrameDumper>,
) -> BreakReason {
    let mut last_frame_at = Instant::now();
    let mut last_window_check = Instant::now();
    let mut next_processed_at = Instant::now();
    let backend = capture.kind();

    loop {
        let snapshot = shared.snapshot();
        if snapshot.shutdown {
            return BreakReason::Shutdown;
        }
        if snapshot.settings.capture != *capture_config
            || snapshot.settings.detection.confirm_frames.max(1) != confirm_frames
        {
            return BreakReason::Restart;
        }

        match capture.next_frame(Duration::from_millis(500)) {
            Ok(Some(frame)) => {
                last_frame_at = Instant::now();

                if !capture.matches_monitor(hwnd) {
                    return BreakReason::Restart;
                }
                if last_window_check.elapsed() >= WINDOW_CHECK_INTERVAL {
                    last_window_check = Instant::now();
                    if let Some(reason) = window_state_changed(hwnd, expected_size) {
                        return reason;
                    }
                }
                if let Some(size) = capture.size() {
                    if frame.size != size {
                        return BreakReason::Restart;
                    }
                }

                // WGC honors SetMinUpdateInterval, but desktop duplication can
                // deliver at the monitor refresh rate. Keep an absolute 60 FPS
                // deadline so a 144 Hz source still averages one processed
                // frame per 16.7 ms instead of dropping every other frame.
                let now = Instant::now();
                if now < next_processed_at {
                    continue;
                }
                next_processed_at = if next_processed_at + CAPTURE_INTERVAL <= now {
                    now + CAPTURE_INTERVAL
                } else {
                    next_processed_at + CAPTURE_INTERVAL
                };

                let scaled = match scaler.downscale(&frame.texture) {
                    Ok(scaled) => scaled,
                    Err(_) => return BreakReason::Lost,
                };

                if scaled.all_black {
                    let now = Instant::now();
                    let since = *runtime.black_since.get_or_insert(now);
                    if since.elapsed() >= Duration::from_secs(3) && backend == Backend::Wgc {
                        return BreakReason::BlackFallback;
                    }
                } else {
                    runtime.black_since = None;
                }

                // An all-black frame cannot contain a loading screen; the
                // detector would scan the whole frame to reach the same answer.
                let detected = !scaled.all_black
                    && detector::detect_loading(&scaled.data, scaled.width, scaled.height);
                let confirmed = runtime.confirm.update(detected, confirm_frames);

                if let Some(dumper) = dumper.as_deref_mut() {
                    dumper.write(&scaled, frame.timestamp, detected, confirmed);
                }

                if confirmed != runtime.loading {
                    runtime.loading = confirmed;
                    shared.set_capture_status(
                        CaptureStatus::Capturing,
                        capture_detail(expected_size),
                    );
                }
                // Also covers the first frame after a restart or a lost window.
                shared.set_desired_paused(runtime.loading);
            }
            Ok(None) => {
                if last_window_check.elapsed() >= WINDOW_CHECK_INTERVAL {
                    last_window_check = Instant::now();
                    if let Some(reason) = window_state_changed(hwnd, expected_size) {
                        return reason;
                    }
                    if !capture.matches_monitor(hwnd) {
                        return BreakReason::Restart;
                    }
                }

                if last_frame_at.elapsed() >= NO_FRAME_TIMEOUT {
                    if backend == Backend::Wgc {
                        return BreakReason::Lost;
                    }
                    // Desktop duplication can legitimately stop delivering
                    // frames for a static screen. WGC restarts; DXGI only has
                    // to protect game time from staying paused forever.
                    if runtime.loading {
                        runtime.loading = false;
                        shared.set_desired_paused(false);
                        shared.set_capture_status(
                            CaptureStatus::Capturing,
                            capture_detail(expected_size),
                        );
                    }
                }
            }
            Err(_) => return BreakReason::Lost,
        }
    }
}

fn window_state_changed(
    hwnd: windows::Win32::Foundation::HWND,
    expected_size: (u32, u32),
) -> Option<BreakReason> {
    if !window::is_capturable(hwnd) {
        return Some(BreakReason::Lost);
    }
    if window::client_size(hwnd)
        .map(|size| size != expected_size)
        .unwrap_or(true)
    {
        return Some(BreakReason::Restart);
    }
    None
}

fn capture_detail(expected_size: (u32, u32)) -> String {
    format!(
        "Diablo II: Resurrected ({}x{})",
        expected_size.0, expected_size.1
    )
}

fn start_capture(
    backend: Backend,
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<ActiveCapture, CaptureError> {
    match backend {
        Backend::Wgc => WgcCapture::start(hwnd, device, CAPTURE_FPS).map(ActiveCapture::Wgc),
        Backend::Dxgi => DxgiCapture::start(device, hwnd).map(ActiveCapture::Dxgi),
    }
}

struct FrameDumper {
    directory: PathBuf,
    index: u64,
    csv: std::fs::File,
}

impl FrameDumper {
    fn new(directory: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("README.txt"),
            "Raw detector frames, tightly packed RGBA (no header), 160 pixels on the long side.\n\
             The short side depends on the game aspect ratio (16:9 = 160x90, 16:10 = 160x100, 4:3 = 160x120).\n\
             frames.csv records the frame timestamp and the detector result. Use these frames to\n\
             inspect the first animation frame that classifies as loading.\n",
        )?;

        let mut csv = std::fs::File::create(directory.join("frames.csv"))?;
        writeln!(
            csv,
            "index,file,timestamp,raw_loading,confirmed_loading,width,height"
        )?;
        Ok(Self {
            directory,
            index: 0,
            csv,
        })
    }

    fn write(&mut self, frame: &ScaledFrame<'_>, timestamp: i64, raw: bool, confirmed: bool) {
        let file = format!(
            "frame-{:06}-{:016}-{}.rgba",
            self.index,
            timestamp as u64,
            if raw { "loading" } else { "gameplay" }
        );
        if std::fs::write(self.directory.join(&file), &frame.data).is_ok() {
            let _ = writeln!(
                self.csv,
                "{},{},{},{},{},{},{}",
                self.index, file, timestamp, raw, confirmed, frame.width, frame.height
            );
            let _ = self.csv.flush();
        }
        self.index = self.index.wrapping_add(1);
    }
}

fn create_device() -> Result<windows::Win32::Graphics::Direct3D11::ID3D11Device, String> {
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };

    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|error| error.to_string())?;

    device.ok_or_else(|| "D3D11 device was not created".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_frames_requires_consecutive_frames() {
        let mut confirm = ConfirmState::new();
        assert!(!confirm.update(true, 3));
        assert!(!confirm.update(true, 3));
        assert!(confirm.update(true, 3));
        assert!(confirm.update(true, 3));

        assert!(confirm.update(false, 3));
        assert!(confirm.update(false, 3));
        assert!(!confirm.update(false, 3));
    }

    #[test]
    fn confirmation_drops_flicker() {
        let mut confirm = ConfirmState::new();
        assert!(!confirm.update(true, 2));
        assert!(!confirm.update(false, 2));
        assert!(!confirm.update(true, 2));
        assert!(confirm.update(true, 2));
    }

    #[test]
    fn black_frame_produces_no_detection() {
        let frame = vec![0u8; 160 * 90 * 4];
        assert!(!detector::detect_loading(&frame, 160, 90));
    }
}
