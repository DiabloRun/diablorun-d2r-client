//! Native Win32 settings dialog built from an in-memory dialog template.

use std::mem::size_of;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetMonitorInfoW, GetObjectW, GetSysColorBrush, HMONITOR,
    LOGFONTW, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MonitorFromPoint,
    MonitorFromWindow, SetBkMode, SetTextColor, COLOR_3DFACE, FW_BOLD, HDC, HFONT, HGDIOBJ,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateDialogIndirectParamW, CreateWindowExW, DestroyWindow, EndDialog,
    GetDlgCtrlID, GetDlgItem, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, KillTimer, LoadIconW, MapDialogRect, MoveWindow, SendMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, CBN_SELCHANGE,
    CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, EN_CHANGE, EN_KILLFOCUS,
    ES_AUTOHSCROLL, GWLP_USERDATA, ICON_BIG, ICON_SMALL, SW_SHOWNORMAL, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_EXITSIZEMOVE, WM_GETFONT, WM_INITDIALOG, WM_NCDESTROY,
    WM_SETFONT, WM_SETICON, WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER, WS_CHILD,
    WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

#[cfg(test)]
use windows::Win32::UI::WindowsAndMessaging::DialogBoxIndirectParamW;

use crate::config::{Config, TransportKind};
use crate::state::{CaptureStatus, LiveSplitStatus, SharedState};
use crate::theme;
use crate::transport;

pub const WM_APP_SETTINGS_SAVED: u32 = 0x8000 + 4;
pub const WM_APP_SETTINGS_CLOSED: u32 = 0x8000 + 5;
pub const WM_APP_SETTINGS_MOVED: u32 = 0x8000 + 6;

const ID_TRANSPORT: i32 = 101;
const ID_HOST: i32 = 102;
const ID_PORT: i32 = 103;
const ID_WS_PATH: i32 = 104;
const ID_TEST: i32 = 106;
const ID_TEST_STATUS: i32 = 107;
const ID_TEST_ICON: i32 = 108;
const ID_LIVESPLIT_STATUS: i32 = 109;
const ID_LIVESPLIT_STATUS_ICON: i32 = 110;
const ID_CAPTURE_STATUS: i32 = 111;
const ID_CAPTURE_STATUS_ICON: i32 = 112;
const ID_CLOSE: i32 = 131;

const ID_CANCEL: i32 = 2;

/// SS_CENTER: center the icon glyph horizontally.
const SS_CENTER: u32 = 0x0001;
/// SS_CENTERIMAGE: vertically center static text inside its control rectangle.
const SS_CENTERIMAGE: u32 = 0x0200;

const STATUS_TIMER_ID: usize = 1;
const STATUS_REFRESH_MS: u32 = 500;
const AUTOSAVE_DELAY: Duration = Duration::from_millis(400);

// Layout in dialog units. Dialog units scale with the dialog font and DPI, so
// these stay proportional across display settings.
const DIALOG_WIDTH: i32 = 270;
const DIALOG_HEIGHT: i32 = 232;
const MARGIN: i32 = 8;
const CONTENT_WIDTH: i32 = DIALOG_WIDTH - 2 * MARGIN;
const SECTION_HEIGHT: i32 = 12;
const STATUS_HEIGHT: i32 = 18;
const STATUS_GAP: i32 = 4;
const GROUP_GAP: i32 = 8;
const SECTION_GAP: i32 = 12;
const ACTION_GAP: i32 = 8;
const FIELD_X: i32 = 60;
const LABEL_WIDTH: i32 = FIELD_X - MARGIN;
const FIELD_WIDTH: i32 = DIALOG_WIDTH - MARGIN - FIELD_X;
const FIELD_HEIGHT: i32 = 14;
const PORT_FIELD_WIDTH: i32 = 62;
const COMBO_OFFSET: i32 = 2;
const COMBO_HEIGHT: i32 = 60;
const BUTTON_HEIGHT: i32 = 14;
const TEST_BUTTON_WIDTH: i32 = 74;
const CLOSE_BUTTON_WIDTH: i32 = 76;
const ICON_SIZE: i32 = 12;
const ICON_TEXT_GAP: i32 = 4;
const STATUS_TEXT_X: i32 = MARGIN + ICON_SIZE + ICON_TEXT_GAP;
const STATUS_TEXT_WIDTH: i32 = DIALOG_WIDTH - MARGIN - STATUS_TEXT_X;
const TEST_STATUS_HEIGHT: i32 = 14;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TestStatus {
    Clear,
    Pending,
    Success,
    Failure,
}

impl TestStatus {
    fn glyph(self) -> &'static str {
        match self {
            Self::Clear => "",
            Self::Pending => "\u{25D4}",
            Self::Success => "\u{2714}",
            Self::Failure => "\u{2716}",
        }
    }

    fn color(self) -> Option<COLORREF> {
        match self {
            Self::Clear => None,
            Self::Pending => Some(COLORREF(0x0080_8080)),
            Self::Success => Some(COLORREF(0x0053_A834)),
            Self::Failure => Some(COLORREF(0x0035_43EA)),
        }
    }
}

struct DialogState {
    config: Config,
    parent: HWND,
    shared: Arc<SharedState>,
    result: Option<Config>,
    modal: bool,
    bold_font: Option<HFONT>,
    icon_font: Option<HFONT>,
    status: TestStatus,
    livesplit_row: TestStatus,
    capture_row: TestStatus,
    dirty_since: Option<Instant>,
}

/// Open the settings dialog without blocking the app's message loop.
pub fn open(parent: HWND, current: &Config, shared: Arc<SharedState>) -> Result<HWND, String> {
    theme::init();
    let state = Box::new(DialogState {
        config: current.clone(),
        parent,
        shared,
        result: None,
        modal: false,
        bold_font: None,
        icon_font: None,
        status: TestStatus::Clear,
        livesplit_row: TestStatus::Clear,
        capture_row: TestStatus::Clear,
        dirty_since: None,
    });
    let state_pointer = Box::into_raw(state);
    let template = build_template();
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    let instance = HINSTANCE(instance.0);

    let result = unsafe {
        CreateDialogIndirectParamW(
            Some(instance),
            template.as_ptr() as *const _,
            None,
            Some(dialog_proc),
            LPARAM(state_pointer as isize),
        )
    };

    match result {
        Ok(hwnd) => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
                let _ = SetActiveWindow(hwnd);
                if let Ok(control) = GetDlgItem(Some(hwnd), ID_TRANSPORT) {
                    let _ = SetFocus(Some(control));
                }
            }
            Ok(hwnd)
        }
        Err(error) => {
            unsafe {
                drop(Box::from_raw(state_pointer));
            }
            Err(error.to_string())
        }
    }
}

/// Show the modal settings dialog. Returns the new config when Save is pressed.
/// Only used by the native dialog tests; the app uses the modeless `open`.
#[cfg(test)]
pub fn show(parent: HWND, current: &Config) -> Option<Config> {
    theme::init();
    let mut state = DialogState {
        config: current.clone(),
        parent,
        shared: SharedState::new(current.clone()),
        result: None,
        modal: true,
        bold_font: None,
        icon_font: None,
        status: TestStatus::Clear,
        livesplit_row: TestStatus::Clear,
        capture_row: TestStatus::Clear,
        dirty_since: None,
    };

    let template = build_template();
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    let instance = HINSTANCE(instance.0);
    let result = unsafe {
        DialogBoxIndirectParamW(
            Some(instance),
            template.as_ptr() as *const _,
            Some(parent),
            Some(dialog_proc),
            LPARAM(&mut state as *mut DialogState as isize),
        )
    };

    let _ = result;
    state.result
}

/// Take the config saved by a modeless dialog. Must be called from the
/// `WM_APP_SETTINGS_SAVED` handler, before the dialog is destroyed.
pub fn take_result(hwnd: HWND) -> Option<Config> {
    unsafe { dialog_state(hwnd).and_then(|state| state.result.take()) }
}

/// Top-left corner of the dialog in physical screen coordinates.
pub fn position(hwnd: HWND) -> Option<(i32, i32)> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some((rect.left, rect.top))
}

/// Move the dialog to its remembered position, clamped to the work area of the
/// monitor containing that position. If the position is missing or no longer on
/// a connected display (for example after unplugging a monitor), center the
/// dialog instead.
fn place_window(hwnd: HWND, state: &DialogState) {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }

    if let (Some(x), Some(y)) = (state.config.window.settings_x, state.config.window.settings_y) {
        let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) };
        if let Some(work) = monitor_work_area(monitor) {
            let (x, y) = clamp_position(x, y, width, height, work);
            unsafe {
                let _ = MoveWindow(hwnd, x, y, width, height, false);
            }
            return;
        }
    }

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if let Some(work) = monitor_work_area(monitor) {
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        unsafe {
            let _ = MoveWindow(hwnd, x, y, width, height, false);
        }
    }
}

fn monitor_work_area(monitor: HMONITOR) -> Option<RECT> {
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        Some(info.rcWork)
    } else {
        None
    }
}

fn clamp_position(x: i32, y: i32, width: i32, height: i32, work: RECT) -> (i32, i32) {
    let max_x = (work.right - width).max(work.left);
    let max_y = (work.bottom - height).max(work.top);
    (x.clamp(work.left, max_x), y.clamp(work.top, max_y))
}

fn notify_moved(hwnd: HWND, state: &DialogState) {
    if state.modal {
        return;
    }
    unsafe {
        let _ = SendMessageW(
            state.parent,
            WM_APP_SETTINGS_MOVED,
            Some(WPARAM(hwnd.0 as usize)),
            Some(LPARAM(0)),
        );
    }
}

fn build_template() -> Vec<u8> {
    let mut template = Vec::new();
    // modal frame, setfont, popup, caption, sysmenu. The dialog is centered
    // manually in `place_window` so a remembered position is not overridden by
    // DS_CENTER when the dialog is shown.
    let style: u32 = 0x80 | 0x40 | 0x8000_0000 | 0x00C0_0000 | 0x0008_0000;
    let ex_style: u32 = 0x0004_0000; // WS_EX_APPWINDOW: show the dialog in the taskbar
    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&0u16.to_le_bytes()); // no items
    template.extend_from_slice(&0i16.to_le_bytes()); // x
    template.extend_from_slice(&0i16.to_le_bytes()); // y
    template.extend_from_slice(&(DIALOG_WIDTH as i16).to_le_bytes()); // cx
    template.extend_from_slice(&(DIALOG_HEIGHT as i16).to_le_bytes()); // cy
    template.extend_from_slice(&0u16.to_le_bytes()); // menu
    template.extend_from_slice(&0u16.to_le_bytes()); // class
    for unit in "D2R LRT Tracker Settings".encode_utf16() {
        template.extend_from_slice(&unit.to_le_bytes());
    }
    template.extend_from_slice(&0u16.to_le_bytes());
    template.extend_from_slice(&9u16.to_le_bytes()); // font size
    for unit in "Segoe UI".encode_utf16() {
        template.extend_from_slice(&unit.to_le_bytes());
    }
    template.extend_from_slice(&0u16.to_le_bytes());
    template
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            let state = unsafe { &mut *(lparam.0 as *mut DialogState) };
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as *mut DialogState as isize);
                let module = GetModuleHandleW(None).unwrap_or_default();
                if let Ok(icon) = LoadIconW(Some(HINSTANCE(module.0)), PCWSTR(1 as *const u16)) {
                    let _ = SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(ICON_BIG as usize)),
                        Some(LPARAM(icon.0 as isize)),
                    );
                    let _ = SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(ICON_SMALL as usize)),
                        Some(LPARAM(icon.0 as isize)),
                    );
                }
            }
            create_controls(hwnd, state);
            place_window(hwnd, state);
            theme::apply(hwnd, theme::is_dark());
            unsafe {
                let _ = SetTimer(Some(hwnd), STATUS_TIMER_ID, STATUS_REFRESH_MS, None);
            }
            LRESULT(1).0
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let state = unsafe { dialog_state(hwnd) };
            let Some(state) = state else {
                return 0;
            };

            match id {
                ID_CLOSE | ID_CANCEL => {
                    save_current(hwnd, state);
                    unsafe {
                        close_dialog(hwnd, state);
                    }
                }
                ID_TEST => {
                    if let Some(config) = read_ui(hwnd, state) {
                        if let Err(error) = config.validate() {
                            set_status(
                                hwnd,
                                state,
                                TestStatus::Failure,
                                &format!("Invalid LiveSplit settings: {error}"),
                            );
                        } else {
                            set_status(
                                hwnd,
                                state,
                                TestStatus::Pending,
                                "Testing LiveSplit connection...",
                            );
                            match transport::test_connection(&config.livesplit) {
                                Ok(kind) => {
                                    let name = match kind {
                                        TransportKind::Pipe => "named pipe",
                                        TransportKind::Tcp => "TCP",
                                        TransportKind::WebSocket => "WebSocket",
                                    };
                                    set_status(
                                        hwnd,
                                        state,
                                        TestStatus::Success,
                                        &format!("LiveSplit connected via {name}"),
                                    );
                                }
                                Err(error) => set_status(hwnd, state, TestStatus::Failure, &error),
                            }
                        }
                    }
                }
                ID_TRANSPORT if code == CBN_SELCHANGE => {
                    update_control_states(hwnd);
                    state.dirty_since = Some(Instant::now());
                }
                ID_HOST | ID_PORT | ID_WS_PATH if code == EN_CHANGE => {
                    state.dirty_since = Some(Instant::now());
                }
                ID_HOST | ID_PORT | ID_WS_PATH if code == EN_KILLFOCUS => {
                    if state.dirty_since.is_some() {
                        save_current(hwnd, state);
                    }
                }
                _ => {}
            }
            0
        }
        WM_TIMER => {
            if wparam.0 == STATUS_TIMER_ID {
                if let Some(state) = unsafe { dialog_state(hwnd) } {
                    update_live_status(hwnd, state);
                    if state
                        .dirty_since
                        .is_some_and(|since| since.elapsed() >= AUTOSAVE_DELAY)
                    {
                        save_current(hwnd, state);
                    }
                }
            }
            0
        }
        WM_EXITSIZEMOVE => {
            if let Some(state) = unsafe { dialog_state(hwnd) } {
                notify_moved(hwnd, state);
            }
            0
        }
        WM_CLOSE => {
            if let Some(state) = unsafe { dialog_state(hwnd) } {
                save_current(hwnd, state);
                unsafe {
                    close_dialog(hwnd, state);
                }
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                let _ = KillTimer(Some(hwnd), STATUS_TIMER_ID);
            }
            if let Some(state) = unsafe { dialog_state(hwnd) } {
                if !state.modal {
                    notify_moved(hwnd, state);
                    unsafe {
                        let _ = SendMessageW(
                            state.parent,
                            WM_APP_SETTINGS_CLOSED,
                            Some(WPARAM(hwnd.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }
                }
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut DialogState;
            if !pointer.is_null() {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let state = &*pointer;
                    if let Some(font) = state.bold_font {
                        let _ = DeleteObject(HGDIOBJ(font.0));
                    }
                    if let Some(font) = state.icon_font {
                        let _ = DeleteObject(HGDIOBJ(font.0));
                    }
                    if !state.modal {
                        drop(Box::from_raw(pointer));
                    }
                }
            }
            0
        }
        message if theme::is_control_color_message(message) => {
            if message == WM_CTLCOLORSTATIC {
                if let Some(state) = unsafe { dialog_state(hwnd) } {
                    if let Some(status) = status_for_control(lparam, state) {
                        return status_icon_color(status, wparam);
                    }
                }
            }
            if let Some(result) = theme::control_color(message, wparam) {
                return result;
            }
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            theme::apply(hwnd, theme::is_dark());
            0
        }
        _ => 0,
    }
}

unsafe fn close_dialog(hwnd: HWND, state: &DialogState) {
    unsafe {
        if state.modal {
            let _ = EndDialog(hwnd, ID_CANCEL as isize);
        } else {
            let _ = DestroyWindow(hwnd);
        }
    }
}

unsafe fn dialog_state(hwnd: HWND) -> Option<&'static mut DialogState> {
    let pointer =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
            as *mut DialogState;
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { &mut *pointer })
    }
}

fn create_controls(hwnd: HWND, state: &mut DialogState) {
    let font =
        unsafe { SendMessageW(hwnd, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as usize;
    let bold_font = create_bold_font(font);
    let section_font = bold_font.map_or(font, |bold| bold.0 as usize);
    state.bold_font = bold_font;
    let icon_font = create_icon_font(font);
    let icon_font_value = icon_font.map_or(font, |icon| icon.0 as usize);
    state.icon_font = icon_font;

    let mut y = MARGIN;
    section(hwnd, "Status", MARGIN, y, CONTENT_WIDTH, section_font);
    y += SECTION_HEIGHT + GROUP_GAP;
    let icon_y = y + (STATUS_HEIGHT - ICON_SIZE) / 2;
    icon_static(
        hwnd,
        ID_LIVESPLIT_STATUS_ICON,
        MARGIN,
        icon_y,
        ICON_SIZE,
        ICON_SIZE,
        icon_font_value,
    );
    text(
        hwnd,
        ID_LIVESPLIT_STATUS,
        "",
        STATUS_TEXT_X,
        y,
        STATUS_TEXT_WIDTH,
        STATUS_HEIGHT,
        font,
    );
    y += STATUS_HEIGHT + STATUS_GAP;
    icon_static(
        hwnd,
        ID_CAPTURE_STATUS_ICON,
        MARGIN,
        y + (STATUS_HEIGHT - ICON_SIZE) / 2,
        ICON_SIZE,
        ICON_SIZE,
        icon_font_value,
    );
    text(
        hwnd,
        ID_CAPTURE_STATUS,
        "",
        STATUS_TEXT_X,
        y,
        STATUS_TEXT_WIDTH,
        STATUS_HEIGHT,
        font,
    );
    y += STATUS_HEIGHT + SECTION_GAP;

    section(hwnd, "LiveSplit", MARGIN, y, CONTENT_WIDTH, section_font);
    y += SECTION_HEIGHT + GROUP_GAP;

    label(
        hwnd,
        "Transport:",
        MARGIN,
        y,
        LABEL_WIDTH,
        FIELD_HEIGHT,
        font,
    );
    combo(
        hwnd,
        ID_TRANSPORT,
        FIELD_X,
        y + COMBO_OFFSET,
        FIELD_WIDTH,
        COMBO_HEIGHT,
        &["Named pipe", "TCP", "WebSocket"],
        match state.config.livesplit.transport {
            TransportKind::Pipe => 0,
            TransportKind::Tcp => 1,
            TransportKind::WebSocket => 2,
        },
        font,
    );
    y += FIELD_HEIGHT + GROUP_GAP;

    label(hwnd, "Host:", MARGIN, y, LABEL_WIDTH, FIELD_HEIGHT, font);
    edit(
        hwnd,
        ID_HOST,
        FIELD_X,
        y,
        FIELD_WIDTH,
        FIELD_HEIGHT,
        &state.config.livesplit.host,
        font,
    );
    y += FIELD_HEIGHT + GROUP_GAP;

    label(hwnd, "Port:", MARGIN, y, LABEL_WIDTH, FIELD_HEIGHT, font);
    edit(
        hwnd,
        ID_PORT,
        FIELD_X,
        y,
        PORT_FIELD_WIDTH,
        FIELD_HEIGHT,
        &state.config.livesplit.port.to_string(),
        font,
    );
    y += FIELD_HEIGHT + GROUP_GAP;

    label(hwnd, "WS path:", MARGIN, y, LABEL_WIDTH, FIELD_HEIGHT, font);
    edit(
        hwnd,
        ID_WS_PATH,
        FIELD_X,
        y,
        FIELD_WIDTH,
        FIELD_HEIGHT,
        &state.config.livesplit.websocket_path,
        font,
    );
    y += FIELD_HEIGHT + ACTION_GAP;

    icon_static(
        hwnd,
        ID_TEST_ICON,
        MARGIN,
        y + (TEST_STATUS_HEIGHT - ICON_SIZE) / 2,
        ICON_SIZE,
        ICON_SIZE,
        icon_font_value,
    );
    text(
        hwnd,
        ID_TEST_STATUS,
        "",
        STATUS_TEXT_X,
        y,
        STATUS_TEXT_WIDTH,
        TEST_STATUS_HEIGHT,
        font,
    );
    y += TEST_STATUS_HEIGHT + ACTION_GAP;

    button(
        hwnd,
        ID_TEST,
        "Test connection",
        MARGIN,
        y,
        TEST_BUTTON_WIDTH,
        BUTTON_HEIGHT,
        font,
    );
    button(
        hwnd,
        ID_CLOSE,
        "Minimize to tray",
        DIALOG_WIDTH - MARGIN - CLOSE_BUTTON_WIDTH,
        y,
        CLOSE_BUTTON_WIDTH,
        BUTTON_HEIGHT,
        font,
    );
    debug_assert_eq!(y + BUTTON_HEIGHT + MARGIN, DIALOG_HEIGHT);

    update_live_status(hwnd, state);
    update_control_states(hwnd);
    unsafe {
        if let Ok(control) = GetDlgItem(Some(hwnd), ID_TRANSPORT) {
            let _ = SetFocus(Some(control));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn section(hwnd: HWND, text_value: &str, x: i32, y: i32, width: i32, font: usize) {
    if let Some(control) = create(
        hwnd,
        w!("STATIC"),
        text_value,
        0,
        x,
        y,
        width,
        SECTION_HEIGHT,
        0,
        0,
    ) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn label(hwnd: HWND, text_value: &str, x: i32, y: i32, width: i32, height: i32, font: usize) {
    if let Some(control) = create(
        hwnd,
        w!("STATIC"),
        text_value,
        0,
        x,
        y,
        width,
        height,
        SS_CENTERIMAGE,
        0,
    ) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn text(hwnd: HWND, id: i32, value: &str, x: i32, y: i32, width: i32, height: i32, font: usize) {
    if let Some(control) = create(
        hwnd,
        w!("STATIC"),
        value,
        id,
        x,
        y,
        width,
        height,
        SS_CENTERIMAGE,
        0,
    ) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn icon_static(hwnd: HWND, id: i32, x: i32, y: i32, width: i32, height: i32, font: usize) {
    if let Some(control) = create(
        hwnd,
        w!("STATIC"),
        "",
        id,
        x,
        y,
        width,
        height,
        SS_CENTER | SS_CENTERIMAGE,
        0,
    ) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn edit(hwnd: HWND, id: i32, x: i32, y: i32, width: i32, height: i32, value: &str, font: usize) {
    if let Some(control) = create(
        hwnd,
        w!("EDIT"),
        value,
        id,
        x,
        y,
        width,
        height,
        WS_TABSTOP.0 | ES_AUTOHSCROLL as u32,
        WS_EX_CLIENTEDGE.0,
    ) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn button(hwnd: HWND, id: i32, value: &str, x: i32, y: i32, width: i32, height: i32, font: usize) {
    if let Some(control) = create(
        hwnd,
        w!("BUTTON"),
        value,
        id,
        x,
        y,
        width,
        height,
        WS_TABSTOP.0,
        0,
    ) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn combo(
    hwnd: HWND,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    items: &[&str],
    selected: usize,
    font: usize,
) {
    if let Some(control) = create(
        hwnd,
        w!("COMBOBOX"),
        "",
        id,
        x,
        y,
        width,
        height,
        WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32,
        0,
    ) {
        set_font(control, font);
        for item in items {
            let wide = crate::text::wide(item);
            unsafe {
                SendMessageW(
                    control,
                    CB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                );
            }
        }
        unsafe {
            SendMessageW(
                control,
                CB_SETCURSEL,
                Some(WPARAM(selected)),
                Some(LPARAM(0)),
            );
        }
    }
}

fn create(
    hwnd: HWND,
    class: PCWSTR,
    text_value: &str,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    style: u32,
    ex_style: u32,
) -> Option<HWND> {
    let mut rect = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    };
    unsafe {
        let _ = MapDialogRect(hwnd, &mut rect);
    }
    let wide = crate::text::wide(text_value);
    unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(ex_style),
            class,
            PCWSTR(wide.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | style,
            ),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(hwnd),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(id as *mut _)),
            None,
            None,
        )
        .ok()
    }
}

fn set_font(control: HWND, font: usize) {
    unsafe {
        SendMessageW(control, WM_SETFONT, Some(WPARAM(font)), Some(LPARAM(1)));
    }
}

fn create_bold_font(font: usize) -> Option<HFONT> {
    if font == 0 {
        return None;
    }
    let mut logfont = LOGFONTW::default();
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(font as *mut _),
            size_of::<LOGFONTW>() as i32,
            Some(&mut logfont as *mut _ as *mut _),
        )
    };
    if copied == 0 {
        return None;
    }
    logfont.lfWeight = FW_BOLD.0 as i32;
    let bold = unsafe { CreateFontIndirectW(&logfont) };
    if bold.0.is_null() {
        None
    } else {
        Some(bold)
    }
}

fn create_icon_font(font: usize) -> Option<HFONT> {
    if font == 0 {
        return None;
    }
    let mut logfont = LOGFONTW::default();
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(font as *mut _),
            size_of::<LOGFONTW>() as i32,
            Some(&mut logfont as *mut _ as *mut _),
        )
    };
    if copied == 0 {
        return None;
    }
    logfont.lfHeight = logfont.lfHeight.saturating_mul(4) / 3;
    let face = crate::text::wide("Segoe UI Symbol");
    let length = face.len().min(logfont.lfFaceName.len());
    logfont.lfFaceName[..length].copy_from_slice(&face[..length]);
    let icon = unsafe { CreateFontIndirectW(&logfont) };
    if icon.0.is_null() {
        None
    } else {
        Some(icon)
    }
}

fn update_control_states(hwnd: HWND) {
    let selection = unsafe {
        let Ok(control) = GetDlgItem(Some(hwnd), ID_TRANSPORT) else {
            return;
        };
        SendMessageW(control, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32
    };

    let is_pipe = selection <= 0;
    let is_ws = selection == 2;
    unsafe {
        if let Ok(control) = GetDlgItem(Some(hwnd), ID_HOST) {
            let _ = EnableWindow(control, !is_pipe);
        }
        if let Ok(control) = GetDlgItem(Some(hwnd), ID_PORT) {
            let _ = EnableWindow(control, !is_pipe);
        }
        if let Ok(control) = GetDlgItem(Some(hwnd), ID_WS_PATH) {
            let _ = EnableWindow(control, is_ws);
        }
    }
}

fn read_ui(hwnd: HWND, state: &DialogState) -> Option<Config> {
    let mut config = state.config.clone();

    let selection = unsafe {
        let control = GetDlgItem(Some(hwnd), ID_TRANSPORT).ok()?;
        SendMessageW(control, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32
    };
    config.livesplit.transport = match selection {
        1 => TransportKind::Tcp,
        2 => TransportKind::WebSocket,
        _ => TransportKind::Pipe,
    };

    config.livesplit.host = get_text(hwnd, ID_HOST).trim().to_string();
    config.livesplit.port = parse_u32(&get_text(hwnd, ID_PORT)).unwrap_or(0);
    config.livesplit.websocket_path = get_text(hwnd, ID_WS_PATH).trim().to_string();

    Some(config)
}

fn get_text(hwnd: HWND, id: i32) -> String {
    let Ok(control) = (unsafe { GetDlgItem(Some(hwnd), id) }) else {
        return String::new();
    };
    let length = unsafe { GetWindowTextLengthW(control) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(control, &mut buffer) };
    if copied <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse().ok()
}

fn set_text(hwnd: HWND, id: i32, value: &str) {
    let wide = crate::text::wide(value);
    unsafe {
        if let Ok(control) = GetDlgItem(Some(hwnd), id) {
            let _ = SetWindowTextW(control, PCWSTR(wide.as_ptr()));
        } else {
            let _ = SetDlgItemTextW(hwnd, id, PCWSTR(wide.as_ptr()));
        }
    }
}

fn save_current(hwnd: HWND, state: &mut DialogState) {
    let Some(config) = read_ui(hwnd, state) else {
        state.dirty_since = None;
        return;
    };
    state.dirty_since = None;
    if config.validate().is_err() || config == state.config {
        return;
    }

    state.config = config.clone();
    state.result = Some(config);
    if !state.modal {
        unsafe {
            let _ = SendMessageW(
                state.parent,
                WM_APP_SETTINGS_SAVED,
                Some(WPARAM(hwnd.0 as usize)),
                Some(LPARAM(0)),
            );
        }
    }
}

fn set_status(hwnd: HWND, state: &mut DialogState, status: TestStatus, value: &str) {
    state.status = status;
    set_status_row(hwnd, ID_TEST_ICON, ID_TEST_STATUS, status, value);
}

fn set_status_row(hwnd: HWND, icon_id: i32, text_id: i32, status: TestStatus, value: &str) {
    set_text(hwnd, text_id, value);
    set_text(hwnd, icon_id, status.glyph());
}

fn update_live_status(hwnd: HWND, state: &mut DialogState) {
    let status = state.shared.status();

    let (livesplit_row, livesplit_text) = match status.livesplit_status {
        LiveSplitStatus::Connected => (TestStatus::Success, "LiveSplit connected".to_string()),
        LiveSplitStatus::Connecting | LiveSplitStatus::Disconnected => {
            (TestStatus::Failure, "LiveSplit disconnected".to_string())
        }
    };
    if state.livesplit_row != livesplit_row || get_text(hwnd, ID_LIVESPLIT_STATUS) != livesplit_text
    {
        state.livesplit_row = livesplit_row;
        set_status_row(
            hwnd,
            ID_LIVESPLIT_STATUS_ICON,
            ID_LIVESPLIT_STATUS,
            livesplit_row,
            &livesplit_text,
        );
    }

    let (capture_row, capture_text) = match status.capture_status {
        CaptureStatus::Capturing => (
            TestStatus::Success,
            format!("Capturing {}", status.capture_detail),
        ),
        CaptureStatus::Searching => (
            TestStatus::Pending,
            "Waiting for Diablo II: Resurrected".to_string(),
        ),
        CaptureStatus::Lost => (
            TestStatus::Failure,
            "Diablo II: Resurrected capture lost".to_string(),
        ),
        CaptureStatus::Error => (
            TestStatus::Failure,
            if status.capture_detail.is_empty() {
                "Diablo II: Resurrected capture error".to_string()
            } else {
                format!(
                    "Diablo II: Resurrected capture error: {}",
                    status.capture_detail
                )
            },
        ),
        CaptureStatus::Off => (
            TestStatus::Failure,
            "Diablo II: Resurrected capture off".to_string(),
        ),
    };
    if state.capture_row != capture_row || get_text(hwnd, ID_CAPTURE_STATUS) != capture_text {
        state.capture_row = capture_row;
        set_status_row(
            hwnd,
            ID_CAPTURE_STATUS_ICON,
            ID_CAPTURE_STATUS,
            capture_row,
            &capture_text,
        );
    }
}

fn status_for_control(lparam: LPARAM, state: &DialogState) -> Option<TestStatus> {
    let control = HWND(lparam.0 as *mut _);
    match unsafe { GetDlgCtrlID(control) } {
        ID_TEST_ICON => Some(state.status),
        ID_LIVESPLIT_STATUS_ICON => Some(state.livesplit_row),
        ID_CAPTURE_STATUS_ICON => Some(state.capture_row),
        _ => None,
    }
}

fn status_icon_color(status: TestStatus, wparam: WPARAM) -> isize {
    let hdc = HDC(wparam.0 as *mut _);
    unsafe {
        let background = theme::control_color(WM_CTLCOLORSTATIC, wparam);
        if let Some(color) = status.color() {
            SetTextColor(hdc, color);
        }
        SetBkMode(hdc, TRANSPARENT);
        background.unwrap_or_else(|| GetSysColorBrush(COLOR_3DFACE).0 as isize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE};

    static DIALOG_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn clamps_positions_to_the_work_area() {
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };

        assert_eq!(clamp_position(-1000, 100, 500, 400, work), (-1000, 100));
        assert_eq!(clamp_position(-100, 100, 500, 400, work), (-500, 100));
        assert_eq!(clamp_position(100, 100, 500, 400, work), (-500, 100));
        assert_eq!(clamp_position(-2500, -100, 500, 400, work), (-1920, 0));
        assert_eq!(clamp_position(-1000, 900, 500, 400, work), (-1000, 640));
        // A window larger than the work area is pinned to its top-left.
        assert_eq!(clamp_position(0, 0, 3000, 2000, work), (-1920, 0));
    }

    #[test]
    fn status_glyphs_match_the_result() {
        assert_eq!(TestStatus::Clear.glyph(), "");
        assert_eq!(TestStatus::Pending.glyph(), "\u{25D4}");
        assert_eq!(TestStatus::Success.glyph(), "\u{2714}");
        assert_eq!(TestStatus::Failure.glyph(), "\u{2716}");
        assert!(TestStatus::Pending.color().is_some());
        assert!(TestStatus::Success.color().is_some());
        assert!(TestStatus::Failure.color().is_some());
        assert!(TestStatus::Clear.color().is_none());
    }

    fn find_dialog() -> Option<HWND> {
        unsafe {
            let hwnd = FindWindowW(w!("#32770"), w!("D2R LRT Tracker Settings")).ok()?;
            // The window exists before WM_INITDIALOG has created its controls.
            GetDlgItem(Some(hwnd), ID_CLOSE).ok()?;
            Some(hwnd)
        }
    }

    #[test]
    #[ignore = "drives a native dialog; run with --ignored --test-threads=1"]
    fn dialog_template_can_be_created_and_cancelled() {
        let _guard = DIALOG_TEST_LOCK.lock().expect("dialog test lock");
        let thread = std::thread::spawn(|| show(HWND::default(), &Config::default()));

        let mut closed = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if let Some(hwnd) = find_dialog() {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                closed = true;
                break;
            }
        }

        assert!(
            closed,
            "settings dialog window was not created; finished={}",
            thread.is_finished()
        );
        assert!(thread.join().expect("dialog thread").is_none());
    }

    #[test]
    #[ignore = "drives a native dialog; run with --ignored --test-threads=1"]
    fn dialog_restores_its_saved_position() {
        let _guard = DIALOG_TEST_LOCK.lock().expect("dialog test lock");
        let expected = (140, 120);
        let mut config = Config::default();
        config.window.settings_x = Some(expected.0);
        config.window.settings_y = Some(expected.1);

        let thread = std::thread::spawn(move || show(HWND::default(), &config));
        let mut dialog = None;
        let mut restored = None;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            if let Some(hwnd) = find_dialog() {
                dialog = Some(hwnd);
                let mut rect = RECT::default();
                unsafe {
                    let _ = GetWindowRect(hwnd, &mut rect);
                }
                if (rect.left, rect.top) == expected {
                    restored = Some((rect.left, rect.top));
                    break;
                }
            }
        }
        if let Some(hwnd) = dialog {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }

        let _ = thread.join().expect("dialog thread");
        assert_eq!(restored, Some(expected));
    }

    #[test]
    #[ignore = "drives a native dialog; run with --ignored --test-threads=1"]
    fn dialog_autosaves_edited_values() {
        let _guard = DIALOG_TEST_LOCK.lock().expect("dialog test lock");
        let thread = std::thread::spawn(|| show(HWND::default(), &Config::default()));

        let mut saved = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if let Some(hwnd) = find_dialog() {
                unsafe {
                    let _ = SetDlgItemTextW(hwnd, ID_HOST, w!("192.168.1.10"));
                }
                std::thread::sleep(std::time::Duration::from_millis(700));
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                saved = true;
                break;
            }
        }

        assert!(saved, "settings dialog window was not created");
        let config = thread.join().expect("dialog thread");
        assert_eq!(config.expect("saved config").livesplit.host, "192.168.1.10");
    }

    #[test]
    #[ignore = "drives a native dialog; run with --ignored --test-threads=1"]
    fn dialog_discards_invalid_edits() {
        let _guard = DIALOG_TEST_LOCK.lock().expect("dialog test lock");
        let thread = std::thread::spawn(|| show(HWND::default(), &Config::default()));

        let mut found = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if let Some(hwnd) = find_dialog() {
                unsafe {
                    let _ = SetDlgItemTextW(hwnd, ID_PORT, w!("0"));
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                found = true;
                break;
            }
        }
        assert!(found, "settings dialog window was not created");
        assert!(thread.join().expect("dialog thread").is_none());
    }
}
