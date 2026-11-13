//! Native Win32 settings dialog built from an in-memory dialog template.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateDialogIndirectParamW, CreateWindowExW, DestroyWindow,
    DialogBoxIndirectParamW, EndDialog, GetDlgItem, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, LoadIconW, MapDialogRect, MessageBoxW, SendMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow, CBS_DROPDOWNLIST,
    CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, ES_AUTOHSCROLL, GWLP_USERDATA, ICON_BIG, ICON_SMALL,
    MB_ICONWARNING, MB_OK, SW_SHOWNORMAL, WM_GETFONT, WM_SETFONT, WM_SETICON, WS_CHILD,
    WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::config::{Config, LiveSplitConfig, TransportKind};
use crate::transport;

pub const WM_APP_SETTINGS_SAVED: u32 = 0x8000 + 4;
pub const WM_APP_SETTINGS_CLOSED: u32 = 0x8000 + 5;

const ID_TRANSPORT: i32 = 101;
const ID_HOST: i32 = 102;
const ID_PORT: i32 = 103;
const ID_WS_PATH: i32 = 104;
const ID_RECONNECT: i32 = 105;
const ID_TEST: i32 = 106;
const ID_TEST_STATUS: i32 = 107;
const ID_SAVE: i32 = 130;
const ID_CANCEL: i32 = 131;

const IDOK: isize = 1;
const IDCANCEL: isize = 2;
const IDCANCEL_ID: i32 = 2;

struct DialogState {
    config: Config,
    parent: HWND,
    result: Option<Config>,
    modal: bool,
}

/// Open the settings dialog without blocking the app's message loop.
pub fn open(parent: HWND, current: &Config) -> Result<HWND, String> {
    let state = Box::new(DialogState {
        config: current.clone(),
        parent,
        result: None,
        modal: false,
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
/// Kept for the native dialog tests; the app uses the modeless `open`.
#[allow(dead_code)]
pub fn show(parent: HWND, current: &Config) -> Option<Config> {
    let mut state = DialogState {
        config: current.clone(),
        parent,
        result: None,
        modal: true,
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

    if result == IDOK {
        state.result
    } else {
        None
    }
}

/// Take the config saved by a modeless dialog. Must be called from the
/// `WM_APP_SETTINGS_SAVED` handler, before the dialog is destroyed.
pub fn take_result(hwnd: HWND) -> Option<Config> {
    unsafe { dialog_state(hwnd).and_then(|state| state.result.take()) }
}

fn build_template() -> Vec<u8> {
    let mut template = Vec::new();
    // modal frame, setfont, shellfont, popup, caption, sysmenu
    let style: u32 = 0x80 | 0x40 | 0x0800 | 0x8000_0000 | 0x00C0_0000 | 0x0008_0000;
    let ex_style: u32 = 0x0004_0000; // WS_EX_APPWINDOW: show the dialog in the taskbar
    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&0u16.to_le_bytes()); // no items
    template.extend_from_slice(&0i16.to_le_bytes()); // x
    template.extend_from_slice(&0i16.to_le_bytes()); // y
    template.extend_from_slice(&340i16.to_le_bytes()); // cx
    template.extend_from_slice(&250i16.to_le_bytes()); // cy
    template.extend_from_slice(&0u16.to_le_bytes()); // menu
    template.extend_from_slice(&0u16.to_le_bytes()); // class
    for unit in "D2R Load Remover Settings".encode_utf16() {
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
        0x0110 => {
            // WM_INITDIALOG
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
            LRESULT(1).0
        }
        0x0111 => {
            // WM_COMMAND
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            let state = unsafe { dialog_state(hwnd) };
            let Some(state) = state else {
                return 0;
            };

            match id {
                ID_SAVE => {
                    if let Some(config) = read_ui(hwnd, state) {
                        if let Err(error) = config.validate() {
                            show_error(hwnd, &error);
                            return 0;
                        }
                        state.result = Some(config);
                        unsafe {
                            if state.modal {
                                let _ = EndDialog(hwnd, IDOK);
                            } else {
                                let parent = state.parent;
                                let _ = SendMessageW(
                                    parent,
                                    WM_APP_SETTINGS_SAVED,
                                    Some(WPARAM(hwnd.0 as usize)),
                                    Some(LPARAM(0)),
                                );
                                let _ = DestroyWindow(hwnd);
                            }
                        }
                    }
                }
                ID_CANCEL | IDCANCEL_ID => unsafe {
                    close_dialog(hwnd, state);
                },
                ID_TEST => {
                    if let Some(config) = read_ui(hwnd, state) {
                        if let Err(error) = config.validate() {
                            set_text(hwnd, ID_TEST_STATUS, &format!("Invalid: {error}"));
                        } else {
                            set_text(hwnd, ID_TEST_STATUS, "Testing...");
                            match transport::test_connection(&config.livesplit) {
                                Ok(transport) => set_text(
                                    hwnd,
                                    ID_TEST_STATUS,
                                    &format!("Connected via {}", transport.as_str()),
                                ),
                                Err(error) => {
                                    set_text(hwnd, ID_TEST_STATUS, &format!("Failed: {error}"))
                                }
                            }
                        }
                    }
                }
                ID_TRANSPORT if code == 1 => update_control_states(hwnd),
                _ => {}
            }
            0
        }
        0x0010 => {
            // WM_CLOSE
            if let Some(state) = unsafe { dialog_state(hwnd) } {
                unsafe {
                    close_dialog(hwnd, state);
                }
            }
            0
        }
        0x0002 => {
            // WM_DESTROY
            if let Some(state) = unsafe { dialog_state(hwnd) } {
                if !state.modal {
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
        0x0082 => {
            // WM_NCDESTROY
            let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut DialogState;
            if !pointer.is_null() {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let state = &*pointer;
                    if !state.modal {
                        drop(Box::from_raw(pointer));
                    }
                }
            }
            0
        }
        _ => 0,
    }
}

unsafe fn close_dialog(hwnd: HWND, state: &DialogState) {
    unsafe {
        if state.modal {
            let _ = EndDialog(hwnd, IDCANCEL);
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

fn create_controls(hwnd: HWND, state: &DialogState) {
    let font =
        unsafe { SendMessageW(hwnd, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as usize;

    section(hwnd, "LiveSplit", 8, 8);
    label(hwnd, "Transport:", 8, 26);
    combo(
        hwnd,
        ID_TRANSPORT,
        95,
        24,
        210,
        60,
        &["Named pipe", "TCP", "WebSocket"],
        match state.config.livesplit.transport {
            TransportKind::Pipe => 0,
            TransportKind::Tcp => 1,
            TransportKind::WebSocket => 2,
        },
        font,
    );

    label(hwnd, "Host:", 8, 52);
    edit(
        hwnd,
        ID_HOST,
        95,
        50,
        210,
        14,
        &state.config.livesplit.host,
        font,
    );
    label(hwnd, "Port:", 8, 74);
    edit(
        hwnd,
        ID_PORT,
        95,
        72,
        80,
        14,
        &state.config.livesplit.port.to_string(),
        font,
    );
    label(hwnd, "WS path:", 8, 96);
    edit(
        hwnd,
        ID_WS_PATH,
        95,
        94,
        210,
        14,
        &state.config.livesplit.websocket_path,
        font,
    );
    label(hwnd, "Reconnect:", 8, 118);
    edit(
        hwnd,
        ID_RECONNECT,
        95,
        116,
        80,
        14,
        &state.config.livesplit.reconnect_ms.to_string(),
        font,
    );
    label(hwnd, "ms", 180, 118);

    button(hwnd, ID_TEST, "Test connection", 8, 140, 82, 18, font);
    text(hwnd, ID_TEST_STATUS, "Not tested", 95, 142, 235, 16, font);

    section(hwnd, "Capture", 8, 176);
    text(hwnd, 0, "Capture FPS: 60 (fixed)", 8, 192, 240, 16, font);

    text(hwnd, 0, "Ready", 8, 220, 150, 16, font);
    button(hwnd, ID_SAVE, "Save & Close", 170, 218, 90, 20, font);
    button(hwnd, ID_CANCEL, "Cancel", 268, 218, 64, 20, font);

    update_control_states(hwnd);
    unsafe {
        if let Ok(control) = GetDlgItem(Some(hwnd), ID_TRANSPORT) {
            let _ = SetFocus(Some(control));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn section(hwnd: HWND, text_value: &str, x: i32, y: i32) {
    let font =
        unsafe { SendMessageW(hwnd, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as usize;
    if let Some(control) = create(hwnd, w!("STATIC"), text_value, 0, x, y, 300, 12, 0, 0) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn label(hwnd: HWND, text_value: &str, x: i32, y: i32) {
    let font =
        unsafe { SendMessageW(hwnd, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as usize;
    if let Some(control) = create(hwnd, w!("STATIC"), text_value, 0, x, y, 85, 12, 0, 0) {
        set_font(control, font);
    }
}

#[allow(clippy::too_many_arguments)]
fn text(hwnd: HWND, id: i32, value: &str, x: i32, y: i32, width: i32, height: i32, font: usize) {
    if let Some(control) = create(hwnd, w!("STATIC"), value, id, x, y, width, height, 0, 0) {
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
            let wide: Vec<u16> = item.encode_utf16().chain([0]).collect();
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
    let wide: Vec<u16> = text_value.encode_utf16().chain([0]).collect();
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

    config.livesplit = LiveSplitConfig {
        transport: config.livesplit.transport,
        host: get_text(hwnd, ID_HOST),
        port: parse_u32(&get_text(hwnd, ID_PORT)).unwrap_or(0),
        websocket_path: get_text(hwnd, ID_WS_PATH),
        reconnect_ms: u64::from(parse_u32(&get_text(hwnd, ID_RECONNECT)).unwrap_or(0)),
        precision_correction: config.livesplit.precision_correction,
    };

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
    let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
    unsafe {
        if let Ok(control) = GetDlgItem(Some(hwnd), id) {
            let _ = SetWindowTextW(control, PCWSTR(wide.as_ptr()));
        } else {
            let _ = SetDlgItemTextW(hwnd, id, PCWSTR(wide.as_ptr()));
        }
    }
}

fn show_error(hwnd: HWND, message: &str) {
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            w!("D2R Load Remover"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, PostMessageW, WM_CLOSE, WM_COMMAND,
    };

    static DIALOG_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn find_dialog() -> Option<HWND> {
        unsafe { FindWindowW(w!("#32770"), w!("D2R Load Remover Settings")).ok() }
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
    fn dialog_saves_edited_values() {
        let _guard = DIALOG_TEST_LOCK.lock().expect("dialog test lock");
        let thread = std::thread::spawn(|| show(HWND::default(), &Config::default()));

        let mut saved = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if let Some(hwnd) = find_dialog() {
                unsafe {
                    let _ = SetDlgItemTextW(hwnd, ID_HOST, w!("192.168.1.10"));
                    let _ =
                        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(ID_SAVE as usize), LPARAM(0));
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
    fn dialog_rejects_invalid_port_without_closing() {
        let _guard = DIALOG_TEST_LOCK.lock().expect("dialog test lock");
        let thread = std::thread::spawn(|| show(HWND::default(), &Config::default()));

        let mut found = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if let Some(hwnd) = find_dialog() {
                unsafe {
                    let _ = SetDlgItemTextW(hwnd, ID_PORT, w!("0"));
                    let _ =
                        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(ID_SAVE as usize), LPARAM(0));
                }
                found = true;
                break;
            }
        }
        assert!(found, "settings dialog window was not created");

        // The validation error opens a message box; close it, then cancel the
        // still-open dialog so the test thread can finish.
        std::thread::sleep(std::time::Duration::from_millis(200));
        unsafe {
            if let Ok(message) = FindWindowW(w!("#32770"), w!("D2R Load Remover")) {
                let _ = PostMessageW(Some(message), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            if let Some(dialog) = find_dialog() {
                let _ = PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }

        assert!(thread.join().expect("dialog thread").is_none());
    }
}
