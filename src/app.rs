//! UI thread, tray integration, lifecycle, and wiring between workers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GetMessageW, GetWindowLongPtrW, IsWindow, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, GWLP_USERDATA, MB_ICONERROR, MB_OK, SW_SHOWNORMAL, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WNDCLASSEXW,
};

use crate::config::{self, Config};
use crate::settings_dialog;
use crate::state::SharedState;
use crate::transport;
use crate::tray::{self, TrayIcon};
use crate::{capture, livesplit};

const WINDOW_CLASS: PCWSTR = w!("D2RLrtTrackerMessageWindow");
const WINDOW_TITLE: PCWSTR = w!("D2R LRT Tracker");
const WM_APP_OPEN_SETTINGS: u32 = 0x8000 + 2;
const WM_APP_SHUTDOWN: u32 = 0x8000 + 3;

static MESSAGE_WINDOW: AtomicIsize = AtomicIsize::new(0);

pub struct Options {
    pub no_tray: bool,
    pub save_frames: Option<PathBuf>,
}

pub fn run(config: Config, config_path: PathBuf, options: Options) -> Result<(), String> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE(instance.0);

    let window_class: Vec<u16> = "D2RLrtTrackerMessageWindow"
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

    let mut app = Box::new(App::new(hwnd, config, config_path));
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app.as_mut() as *mut App as isize);
    }
    app.start(&options);

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
    tray: Option<TrayIcon>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    livesplit_thread: Option<std::thread::JoinHandle<()>>,
    stopped: bool,
    settings_hwnd: Option<HWND>,
}

impl App {
    fn new(hwnd: HWND, config: Config, config_path: PathBuf) -> Self {
        let shared = SharedState::new(config.clone());
        Self {
            hwnd,
            shared,
            config,
            config_path,
            tray: None,
            capture_thread: None,
            livesplit_thread: None,
            stopped: false,
            settings_hwnd: None,
        }
    }

    fn start(&mut self, options: &Options) {
        let capture_shared = self.shared.clone();
        let save_frames = options.save_frames.clone();
        match std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || capture::run(capture_shared, save_frames))
        {
            Ok(thread) => self.capture_thread = Some(thread),
            Err(error) => show_error_message(
                self.hwnd,
                &format!("Could not start capture thread: {error}"),
            ),
        }

        let livesplit_shared = self.shared.clone();
        let connector: Arc<livesplit::Connector> =
            Arc::new(|settings| transport::connect(settings));
        match std::thread::Builder::new()
            .name("livesplit".into())
            .spawn(move || livesplit::run(livesplit_shared, connector))
        {
            Ok(thread) => self.livesplit_thread = Some(thread),
            Err(error) => show_error_message(
                self.hwnd,
                &format!("Could not start LiveSplit thread: {error}"),
            ),
        }

        if !options.no_tray {
            let mut tray = TrayIcon::new(self.hwnd);
            tray.add();
            self.tray = Some(tray);

            // Show the connection settings immediately on a normal launch.
            // Headless `--no-tray` runs skip this along with the tray icon.
            self.open_settings();
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.close_settings();
        self.shared.request_shutdown();

        if let Some(tray) = self.tray.as_mut() {
            tray.remove();
        }

        if let Some(thread) = self.livesplit_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
    }

    fn open_settings(&mut self) {
        if let Some(hwnd) = self.settings_hwnd {
            if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
                    let _ = BringWindowToTop(hwnd);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetActiveWindow(hwnd);
                }
                return;
            }
            self.settings_hwnd = None;
        }

        match settings_dialog::open(self.hwnd, &self.config, Arc::clone(&self.shared)) {
            Ok(hwnd) => self.settings_hwnd = Some(hwnd),
            Err(error) => show_error_message(
                self.hwnd,
                &format!("Could not open the settings dialog: {error}"),
            ),
        }
    }

    fn close_settings(&mut self) {
        if let Some(hwnd) = self.settings_hwnd.take() {
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    fn apply_settings(&mut self, mut new_config: Config) {
        new_config.normalize();
        if let Err(error) = new_config.validate() {
            show_error_message(self.hwnd, &format!("Invalid settings: {error}"));
            return;
        }

        self.shared.apply_settings(new_config.clone());
        self.config = new_config;
        self.save_config();
    }

    fn save_config(&self) {
        if let Err(error) = config::save_config(&self.config_path, &self.config) {
            show_error_message(self.hwnd, &format!("Could not save config: {error}"));
        }
    }

    fn show_tray_menu(&mut self) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };
        let command = tray.show_menu();
        match command {
            tray::MENU_SETTINGS => self.open_settings(),
            tray::MENU_QUIT => unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            _ => {}
        }
    }

    fn taskbar_created(&mut self) {
        if let Some(tray) = self.tray.as_mut() {
            tray.recreate();
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
            // WM_LBUTTONUP / WM_RBUTTONUP
            if event == 0x0205 || event == 0x0202 {
                app.show_tray_menu();
                return LRESULT(0);
            }
        }

        if message == WM_APP_OPEN_SETTINGS {
            app.open_settings();
            return LRESULT(0);
        }

        if message == settings_dialog::WM_APP_SETTINGS_SAVED {
            let dialog = HWND(wparam.0 as *mut _);
            if let Some(config) = settings_dialog::take_result(dialog) {
                app.apply_settings(config);
            }
            return LRESULT(0);
        }

        if message == settings_dialog::WM_APP_SETTINGS_CLOSED {
            let dialog = HWND(wparam.0 as *mut _);
            if app.settings_hwnd == Some(dialog) {
                app.settings_hwnd = None;
            }
            return LRESULT(0);
        }

        if message == WM_APP_SHUTDOWN {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
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

fn show_error_message(owner: HWND, message: &str) {
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = MessageBoxW(
            Some(owner),
            PCWSTR(wide.as_ptr()),
            w!("D2R LRT Tracker"),
            MB_OK | MB_ICONERROR,
        );
    }
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
