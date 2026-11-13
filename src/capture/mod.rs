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

pub struct CapturedTexture {
    pub texture: ID3D11Texture2D,
    pub timestamp: i64,
    pub size: (u32, u32),
    /// WGC frame objects must stay alive until the texture has been copied.
    pub _keepalive: Option<Direct3D11CaptureFrame>,
}

#[derive(Debug)]
pub enum CaptureError {
    Recoverable(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recoverable(message) => f.write_str(message),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Wgc,
    Dxgi,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Self::Wgc => "WGC",
            Self::Dxgi => "DXGI",
        }
    }
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

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Wgc(capture) => capture.size(),
            Self::Dxgi(_) => (0, 0),
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
    Disabled,
    Shutdown,
    BlackFallback,
}

struct Runtime {
    confirm: ConfirmState,
    loading: bool,
    had_capture: bool,
    hint: Option<String>,
    black_since: Option<Instant>,
    lost_logged: bool,
}

struct ConfirmState {
    stable: bool,
    candidate: Option<(bool, i64, u32)>,
    last_transition: Option<(bool, i64)>,
}

impl ConfirmState {
    fn new() -> Self {
        Self {
            stable: false,
            candidate: None,
            last_transition: None,
        }
    }

    fn reset(&mut self) {
        self.stable = false;
        self.candidate = None;
        self.last_transition = None;
    }

    /// Returns the confirmed state. Requiring consecutive frames preserves the
    /// timestamp of the first candidate frame for future precision work.
    fn update(&mut self, value: bool, timestamp: i64, required: u32) -> bool {
        if value == self.stable {
            self.candidate = None;
            return self.stable;
        }

        let required = required.max(1);
        match self.candidate {
            Some((candidate, first_timestamp, count)) if candidate == value => {
                let count = count + 1;
                if count >= required {
                    self.stable = value;
                    self.candidate = None;
                    self.last_transition = Some((value, first_timestamp));
                } else {
                    self.candidate = Some((candidate, first_timestamp, count));
                }
            }
            _ => {
                if required <= 1 {
                    self.stable = value;
                    self.candidate = None;
                    self.last_transition = Some((value, timestamp));
                } else {
                    self.candidate = Some((value, timestamp, 1));
                }
            }
        }

        self.stable
    }
}

pub fn run(shared: Arc<SharedState>, save_frames: Option<PathBuf>) {
    let mut dumper = match save_frames {
        Some(directory) => match FrameDumper::new(directory) {
            Ok(dumper) => {
                log_info!("Saving detector frames to {}", dumper.directory.display());
                Some(dumper)
            }
            Err(error) => {
                log_warn!("Could not create frame dump directory: {error}");
                None
            }
        },
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
            shared.set_capture_status(CaptureStatus::Error, error.clone());
            shared.set_desired_paused(false);
            log_error!("Could not create D3D11 device: {error}");
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
            log_error!("Could not create downscaler: {error}");
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
        hint: None,
        black_since: None,
        lost_logged: false,
    };
    let mut backend = Backend::Wgc;
    let mut wgc_failures = 0u32;
    let mut last_capture_config: Option<CaptureConfig> = None;

    'main: loop {
        let snapshot = shared.snapshot();
        if snapshot.shutdown {
            break;
        }

        if !snapshot.enabled {
            runtime.confirm.reset();
            runtime.loading = false;
            runtime.had_capture = false;
            runtime.hint = None;
            runtime.black_since = None;
            runtime.lost_logged = false;
            shared.set_desired_paused(false);
            shared.set_capture_status(CaptureStatus::Disabled, "");
            shared.set_hint(None);
            shared.wait_for_revision(snapshot.revision, Duration::from_millis(200));
            continue;
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
                shared.set_hint(runtime.hint.clone());
                shared.set_desired_paused(capture_lost_should_pause(
                    &capture_config,
                    runtime.had_capture,
                ));
                if !runtime.lost_logged {
                    log_warn!(
                        "D2R window lost; {} game time",
                        if capture_config.pause_on_capture_lost {
                            "pausing"
                        } else {
                            "not pausing"
                        }
                    );
                    runtime.lost_logged = true;
                }
            } else {
                shared.set_capture_status(CaptureStatus::Searching, "waiting for D2R");
                shared.set_desired_paused(false);
            }

            shared.wait_for_revision(snapshot.revision, Duration::from_millis(500));
            continue;
        };

        let expected_size = (found.client_width, found.client_height);
        let mut capture = match start_capture(backend, &device, found.hwnd, capture_config.fps) {
            Ok(capture) => capture,
            Err(error) => {
                if backend == Backend::Wgc {
                    wgc_failures += 1;
                    log_warn!("WGC failed ({}): {error}", wgc_failures);
                    backend = Backend::Dxgi;
                    continue;
                }

                log_warn!("Capture start failed: {error}");
                shared.set_capture_status(CaptureStatus::Error, format!("capture error: {error}"));
                if runtime.had_capture {
                    shared.set_desired_paused(capture_lost_should_pause(
                        &capture_config,
                        runtime.had_capture,
                    ));
                }
                shared.wait_for_revision(snapshot.revision, Duration::from_secs(2));
                continue;
            }
        };

        runtime.had_capture = true;
        runtime.lost_logged = false;
        shared.set_capture_status(
            CaptureStatus::Capturing,
            format!(
                "D2R.exe {}x{} via {} ({})",
                expected_size.0,
                expected_size.1,
                capture.kind().label(),
                if runtime.loading {
                    "loading"
                } else {
                    "playing"
                }
            ),
        );
        log_info!(
            "Capturing D2R window {}x{} via {}",
            expected_size.0,
            expected_size.1,
            capture.kind().label()
        );

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
            BreakReason::Disabled => {
                runtime.confirm.reset();
                runtime.loading = false;
                runtime.had_capture = false;
                shared.set_desired_paused(false);
                shared.set_capture_status(CaptureStatus::Disabled, "");
                continue;
            }
            BreakReason::Shutdown => break 'main,
            BreakReason::Restart => continue,
            BreakReason::BlackFallback => {
                if capture.kind() == Backend::Wgc {
                    wgc_failures += 1;
                    if wgc_failures >= 3 {
                        backend = Backend::Dxgi;
                    }
                }
                if backend == Backend::Wgc {
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
                        log_warn!("WGC failed repeatedly; switching to DXGI fallback");
                        backend = Backend::Dxgi;
                    }
                }
                shared.set_capture_status(CaptureStatus::Lost, "capture lost");
                if runtime.had_capture {
                    shared.set_desired_paused(capture_lost_should_pause(
                        &capture_config,
                        runtime.had_capture,
                    ));
                }
                shared.wait_for_revision(shared.snapshot().revision, Duration::from_millis(200));
            }
        }
    }

    shared.set_desired_paused(false);
    shared.set_capture_status(CaptureStatus::Disabled, "");
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
    let backend = capture.kind();

    loop {
        let snapshot = shared.snapshot();
        if snapshot.shutdown {
            return BreakReason::Shutdown;
        }
        if !snapshot.enabled {
            return BreakReason::Disabled;
        }
        if snapshot.settings.capture != *capture_config
            || snapshot.settings.detection.confirm_frames.max(1) != confirm_frames
        {
            return BreakReason::Restart;
        }

        match capture.next_frame(Duration::from_millis(500)) {
            Ok(Some(frame)) => {
                last_frame_at = Instant::now();

                if backend == Backend::Wgc && frame.size != capture.size() {
                    log_info!(
                        "Capture size changed from {:?} to {:?}; restarting",
                        capture.size(),
                        frame.size
                    );
                    return BreakReason::Restart;
                }

                let scaled = match scaler.downscale(&frame.texture) {
                    Ok(scaled) => scaled,
                    Err(error) => {
                        log_warn!("Downscale failed: {error}");
                        return BreakReason::Lost;
                    }
                };

                if scaled.all_black {
                    let now = Instant::now();
                    let since = *runtime.black_since.get_or_insert(now);
                    if since.elapsed() >= Duration::from_secs(3) && backend == Backend::Wgc {
                        let message = "Captured frames are black - switch D2R to Windowed or another Fullscreen mode";
                        if runtime.hint.as_deref() != Some(message) {
                            runtime.hint = Some(message.to_string());
                            shared.set_hint(Some(message.to_string()));
                            log_warn!("Captured frames are black; falling back to DXGI");
                        }
                        return BreakReason::BlackFallback;
                    }
                } else {
                    runtime.black_since = None;
                    if runtime.hint.is_some() {
                        runtime.hint = None;
                        shared.set_hint(None);
                    }
                }

                let detected = detector::detect_loading(&scaled.data, scaled.width, scaled.height);
                let confirmed = runtime
                    .confirm
                    .update(detected, frame.timestamp, confirm_frames);

                if let Some(dumper) = dumper.as_deref_mut() {
                    dumper.write(&scaled, frame.timestamp, detected, confirmed);
                }

                if confirmed != runtime.loading {
                    runtime.loading = confirmed;
                    if confirmed {
                        log_info!("Loading screen detected");
                    } else {
                        log_info!("Gameplay detected");
                    }
                    shared.set_capture_status(
                        CaptureStatus::Capturing,
                        format!(
                            "D2R.exe {}x{} via {} ({})",
                            expected_size.0,
                            expected_size.1,
                            backend.label(),
                            if confirmed { "loading" } else { "playing" }
                        ),
                    );
                    if let Some((state, timestamp)) = runtime.confirm.last_transition {
                        log_debug!(
                            "Detector transition at frame timestamp {} (loading={state})",
                            timestamp
                        );
                    }
                }
                // Also covers the first frame after a restart or a lost window.
                shared.set_desired_paused(runtime.loading);
            }
            Ok(None) => {
                if last_window_check.elapsed() >= Duration::from_secs(2) {
                    last_window_check = Instant::now();
                    if !window::is_capturable(hwnd) {
                        return BreakReason::Lost;
                    }
                    if window::client_size(hwnd)
                        .map(|size| size != expected_size)
                        .unwrap_or(true)
                    {
                        return BreakReason::Restart;
                    }
                }

                if backend == Backend::Wgc && last_frame_at.elapsed() >= Duration::from_secs(10) {
                    log_warn!("WGC stopped delivering frames");
                    return BreakReason::Lost;
                }
            }
            Err(error) => {
                log_warn!("Capture error: {error}");
                return BreakReason::Lost;
            }
        }
    }
}

fn capture_lost_should_pause(config: &CaptureConfig, had_capture: bool) -> bool {
    had_capture && config.pause_on_capture_lost
}

fn start_capture(
    backend: Backend,
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    hwnd: windows::Win32::Foundation::HWND,
    fps: u32,
) -> Result<ActiveCapture, CaptureError> {
    match backend {
        Backend::Wgc => WgcCapture::start(hwnd, device, fps).map(ActiveCapture::Wgc),
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
        if let Err(error) = std::fs::write(self.directory.join(&file), &frame.data) {
            log_warn!("Could not write frame dump: {error}");
        } else {
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
        assert!(!confirm.update(true, 10, 3));
        assert!(!confirm.update(true, 20, 3));
        assert!(confirm.update(true, 30, 3));
        assert_eq!(confirm.last_transition, Some((true, 10)));
        assert!(confirm.update(true, 40, 3));

        assert!(confirm.update(false, 50, 3));
        assert!(confirm.update(false, 60, 3));
        assert!(!confirm.update(false, 70, 3));
        assert_eq!(confirm.last_transition, Some((false, 50)));
    }

    #[test]
    fn confirmation_drops_flicker() {
        let mut confirm = ConfirmState::new();
        assert!(!confirm.update(true, 10, 2));
        assert!(!confirm.update(false, 20, 2));
        assert!(!confirm.update(true, 30, 2));
        assert!(confirm.update(true, 40, 2));
        assert_eq!(confirm.last_transition, Some((true, 30)));
    }

    #[test]
    fn black_frame_produces_no_detection() {
        let frame = vec![0u8; 160 * 90 * 4];
        assert!(!detector::detect_loading(&frame, 160, 90));
    }

    #[test]
    fn capture_loss_only_pauses_after_capture_has_started() {
        let mut config = CaptureConfig::default();
        assert!(capture_lost_should_pause(&config, true));
        assert!(!capture_lost_should_pause(&config, false));

        config.pause_on_capture_lost = false;
        assert!(!capture_lost_should_pause(&config, true));
    }
}
