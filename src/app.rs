//! UI thread, tray integration, lifecycle, and wiring between workers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetMessageW,
    GetWindowLongPtrW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassExW, SetTimer,
    SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, SW_SHOWNORMAL, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WM_HOTKEY, WM_TIMER, WNDCLASSEXW,
};

use crate::config::{self, Config};
use crate::hotkeys;
use crate::logging;
use crate::settings_dialog;
use crate::state::SharedState;
use crate::transport;
use crate::tray::{self, TrayIcon};
use crate::{autostart, capture, livesplit};

const WINDOW_CLASS: PCWSTR = w!("D2RLoadRemoverMessageWindow");
const WINDOW_TITLE: PCWSTR = w!("D2R Load Remover");
const WM_APP_OPEN_SETTINGS: u32 = 0x8000 + 2;
const WM_APP_SHUTDOWN: u32 = 0x8000 + 3;
const HOTKEY_ID: i32 = 1;
const TIMER_ID: usize = 1;

static MESSAGE_WINDOW: AtomicIsize = AtomicIsize::new(0);

pub struct Options {
    pub no_tray: bool,
    pub save_frames: Option<PathBuf>,
}

pub fn run(config: Config, config_path: PathBuf, options: Options) -> Result<(), String> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE(instance.0);

    let window_class: Vec<u16> = "D2RLoadRemoverMessageWindow"
        .encode_utf16()
        .chain([0])
        .collect();
    let class_name = PCWSTR(window_class.as_ptr());

    let window_class_struct = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class_name,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&window_class_struct) } == 0 {
        return Err("could not register the message window class".into());
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            WINDOW_TITLE,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|error| format!("could not create the message window: {error}"))?;

    MESSAGE_WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);

    let mut app = Box::new(App::new(hwnd, config, config_path, options));
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app.as_mut() as *mut App as isize);
    }
    app.start();

    let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    MESSAGE_WINDOW.store(0, Ordering::SeqCst);
    app.stop();
    drop(app);
    Ok(())
}

struct App {
    hwnd: HWND,
    shared: Arc<SharedState>,
    config: Config,
    config_path: PathBuf,
    options: Options,
    tray: Option<TrayIcon>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    livesplit_thread: Option<std::thread::JoinHandle<()>>,
    hotkey_registered: bool,
    stopped: bool,
    settings_open: bool,
}

impl App {
    fn new(hwnd: HWND, config: Config, config_path: PathBuf, options: Options) -> Self {
        let shared = SharedState::new(config.clone());
        Self {
            hwnd,
            shared,
            config,
            config_path,
            options,
            tray: None,
            capture_thread: None,
            livesplit_thread: None,
            hotkey_registered: false,
            stopped: false,
            settings_open: false,
        }
    }

    fn start(&mut self) {
        let capture_shared = self.shared.clone();
        let save_frames = self.options.save_frames.clone();
        match std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || capture::run(capture_shared, save_frames))
        {
            Ok(thread) => self.capture_thread = Some(thread),
            Err(error) => log_error!("Could not start capture thread: {error}"),
        }

        let livesplit_shared = self.shared.clone();
        let connector: Arc<livesplit::Connector> =
            Arc::new(|settings| transport::connect(settings));
        match std::thread::Builder::new()
            .name("livesplit".into())
            .spawn(move || livesplit::run(livesplit_shared, connector))
        {
            Ok(thread) => self.livesplit_thread = Some(thread),
            Err(error) => log_error!("Could not start LiveSplit thread: {error}"),
        }

        if !self.options.no_tray {
            let mut tray = TrayIcon::new(self.hwnd);
            tray.add(&self.shared.tooltip());
            self.tray = Some(tray);
        }

        self.apply_hotkey();
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TIMER_ID, 500, None);
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.shared.request_shutdown();
        self.shared.notify_all();

        if let Some(tray) = self.tray.as_mut() {
            tray.remove();
        }
        self.unregister_hotkey();
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
        }

        if let Some(thread) = self.livesplit_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
    }

    fn apply_hotkey(&mut self) {
        self.unregister_hotkey();
        if self.config.hotkeys.toggle.trim().is_empty() {
            return;
        }

        match hotkeys::parse_hotkey(&self.config.hotkeys.toggle) {
            Ok(hotkey) => match hotkeys::register(self.hwnd, HOTKEY_ID, hotkey) {
                Ok(()) => {
                    self.hotkey_registered = true;
                    log_info!("Registered hotkey {}", self.config.hotkeys.toggle);
                }
                Err(error) => {
                    log_warn!(
                        "Could not register hotkey {}: {error}",
                        self.config.hotkeys.toggle
                    );
                }
            },
            Err(error) => log_warn!("Invalid hotkey: {error}"),
        }
    }

    fn unregister_hotkey(&mut self) {
        if self.hotkey_registered {
            hotkeys::unregister(self.hwnd, HOTKEY_ID);
            self.hotkey_registered = false;
        }
    }

    fn toggle_enabled(&mut self) {
        let enabled = self.shared.toggle_enabled();
        self.config.enabled = enabled;
        self.save_config();
        log_info!(
            "Load removal {}",
            if enabled { "enabled" } else { "disabled" }
        );
        self.update_tooltip();
    }

    fn open_settings(&mut self) {
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        let result = settings_dialog::show(self.hwnd, &self.config);
        self.settings_open = false;

        if let Some(mut new_config) = result {
            new_config.normalize();
            if let Err(error) = new_config.validate() {
                log_warn!("Rejected settings: {error}");
                return;
            }

            let hotkey_changed = new_config.hotkeys.toggle != self.config.hotkeys.toggle;
            self.shared.apply_settings(new_config.clone());
            self.config = new_config;
            self.save_config();
            if hotkey_changed {
                self.apply_hotkey();
            }
            log_info!("Settings applied");
            self.update_tooltip();
        }
    }

    fn save_config(&self) {
        if let Err(error) = config::save_config(&self.config_path, &self.config) {
            log_warn!("Could not save config: {error}");
        }
    }

    fn update_tooltip(&mut self) {
        if let Some(tray) = self.tray.as_mut() {
            tray.set_tooltip(&self.shared.tooltip());
        }
    }

    fn open_path(&self, path: &Path) {
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain([0]).collect();
        unsafe {
            let result = ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
            if result.0 as isize <= 32 {
                log_warn!("Could not open {}", path.display());
            }
        }
    }

    fn open_log(&self) {
        let path = logging::log_path(&self.config_path);
        self.open_path(&path);
    }

    fn open_config_folder(&self) {
        let folder = self
            .config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        self.open_path(&folder);
    }

    fn toggle_autostart(&mut self) {
        match autostart::toggle() {
            Ok(enabled) => log_info!("Autostart {}", if enabled { "enabled" } else { "disabled" }),
            Err(error) => log_warn!("Could not change autostart: {error}"),
        }
    }

    fn show_tray_menu(&mut self) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };
        let command = tray.show_menu(self.shared.enabled(), autostart::is_enabled());
        match command {
            tray::MENU_TOGGLE => self.toggle_enabled(),
            tray::MENU_SETTINGS => self.open_settings(),
            tray::MENU_RECONNECT => {
                self.shared.request_reconnect();
                log_info!("Reconnect requested");
            }
            tray::MENU_OPEN_LOG => self.open_log(),
            tray::MENU_OPEN_CONFIG => self.open_config_folder(),
            tray::MENU_AUTOSTART => self.toggle_autostart(),
            tray::MENU_QUIT => unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            _ => {}
        }
    }

    fn taskbar_created(&mut self) {
        if let Some(tray) = self.tray.as_mut() {
            tray.recreate(&self.shared.tooltip());
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let app_pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    let app = if app_pointer.is_null() {
        None
    } else {
        Some(unsafe { &mut *app_pointer })
    };

    if let Some(app) = app {
        if message == tray::TRAY_CALLBACK_MESSAGE {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == 0x0205 || event == 0x0204 || event == 0x0202 {
                // WM_RBUTTONUP / WM_RBUTTONDOWN / WM_LBUTTONUP
                if !app.settings_open {
                    app.show_tray_menu();
                }
                return LRESULT(0);
            }
        }

        if message == WM_APP_OPEN_SETTINGS {
            app.open_settings();
            return LRESULT(0);
        }

        if message == WM_APP_SHUTDOWN {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }

        if message == WM_TIMER && wparam.0 == TIMER_ID {
            app.update_tooltip();
            return LRESULT(0);
        }

        if message == WM_HOTKEY && wparam.0 as i32 == HOTKEY_ID && !app.settings_open {
            app.toggle_enabled();
            return LRESULT(0);
        }

        if let Some(tray_message) = app.tray.as_ref().map(|tray| tray.taskbar_created_message()) {
            if tray_message != 0 && message == tray_message {
                app.taskbar_created();
                return LRESULT(0);
            }
        }

        if message == WM_CLOSE {
            app.stop();
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }

        if message == WM_ENDSESSION && wparam.0 != 0 {
            app.stop();
        }
    }

    if message == WM_DESTROY {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }

    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

pub fn post_shutdown_from_console() {
    let hwnd = MESSAGE_WINDOW.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                WM_APP_SHUTDOWN,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// Find the running instance's hidden message window.
pub fn find_message_window() -> Option<HWND> {
    unsafe { FindWindowW(WINDOW_CLASS, PCWSTR::null()).ok() }
}

pub fn request_open_settings(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_OPEN_SETTINGS, WPARAM(0), LPARAM(0));
    }
}
