// Ship as a GUI executable so double-clicking the release build does not open
// a console window. Debug builds and test harnesses keep the console subsystem.
#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod app;
mod capture;
mod config;
mod detector;
mod livesplit;
mod replay;
mod settings_dialog;
mod state;
mod theme;
mod transport;
mod tray;

use std::path::PathBuf;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Console::{
    AllocConsole, AttachConsole, GetConsoleWindow, GetStdHandle, SetConsoleCtrlHandler,
    SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

const MUTEX_NAME: PCWSTR = w!("Local\\d2r-lrt-tracker");

struct Cli {
    no_tray: bool,
    save_frames: Option<PathBuf>,
    replay_frames: Option<PathBuf>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let console_requested = args.iter().any(|arg| arg == "--console");
    let needs_console = console_requested
        || args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--help" | "-h" | "--version" | "--replay-frames"
            )
        });
    let has_console = needs_console && ensure_console(console_requested);

    if let Err(error) = run() {
        let is_tool_command = args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--replay-frames" | "--help" | "-h" | "--version"
            )
        });
        if has_console || is_tool_command {
            eprintln!("{error}");
        } else {
            show_startup_error(&error);
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = parse_args()?;

    if let Some(directory) = cli.replay_frames.as_deref() {
        return replay::run(directory);
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    theme::init();

    let loaded = config::load_config();
    let config_path = loaded.path.clone();
    let config = loaded.config;
    if let Some(error) = loaded.error {
        eprintln!("{error}");
    }
    if config_path.parent().is_some_and(|parent| !parent.exists()) {
        let _ = std::fs::create_dir_all(config_path.parent().unwrap());
    }
    if !config_path.exists() {
        let _ = config::save_config(&config_path, &config);
    }

    // Keep the mutex handle alive for the lifetime of the process.
    let mutex = match unsafe { CreateMutexW(None, true, MUTEX_NAME) } {
        Ok(handle) => handle,
        Err(_) => Default::default(),
    };
    let _mutex = mutex;
    if !mutex.is_invalid() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Some(existing) = app::find_message_window() {
            app::request_open_settings(existing);
        }
        return Ok(());
    }

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(console_ctrl_handler), true);
    }

    let options = app::Options {
        no_tray: cli.no_tray,
        save_frames: cli.save_frames,
    };

    app::run(config, config_path, options)
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        no_tray: false,
        save_frames: None,
        replay_frames: None,
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // The console is attached in `main` before startup output.
            "--console" => {}
            "--no-tray" => cli.no_tray = true,
            "--save-frames" => {
                cli.save_frames =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        "--save-frames requires a directory".to_string()
                    })?));
            }
            "--replay-frames" => {
                cli.replay_frames =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        "--replay-frames requires a directory".to_string()
                    })?));
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--version" => {
                println!("d2r-lrt-tracker {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(cli)
}

fn print_usage() {
    println!(
        concat!(
            "d2r-lrt-tracker {}\n\n",
            "Usage: d2r-lrt-tracker [options]\n\n",
            "Options:\n",
            "  --console              Attach a console for development output\n",
            "  --save-frames <dir>    Dump raw detector frames for fixture creation\n",
            "  --replay-frames <dir>  Replay a saved frame dump through the detector\n",
            "  --no-tray              Run headless without a tray icon\n",
            "  --help                 Show this help\n",
            "  --version              Show the version"
        ),
        env!("CARGO_PKG_VERSION")
    );
}

fn ensure_console(allow_allocate: bool) -> bool {
    unsafe {
        if !GetConsoleWindow().is_invalid() {
            return true;
        }

        let attached = AttachConsole(ATTACH_PARENT_PROCESS).is_ok()
            || (allow_allocate && AllocConsole().is_ok());
        if !attached {
            return false;
        }

        reconnect_console_handles();
        true
    }
}

/// A GUI-subsystem process starts without standard handles even after attaching
/// to a console, so point the missing ones at the console it is now using.
fn reconnect_console_handles() {
    unsafe {
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE).ok();
        let stderr = GetStdHandle(STD_ERROR_HANDLE).ok();
        let missing = |handle: Option<HANDLE>| handle.is_none_or(|handle| handle.is_invalid());
        if !missing(stdout) && !missing(stderr) {
            return;
        }

        let Ok(conout) = CreateFileW(
            w!("CONOUT$"),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        ) else {
            return;
        };

        if missing(stdout) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, conout);
        }
        if missing(stderr) {
            let _ = SetStdHandle(STD_ERROR_HANDLE, conout);
        }
    }
}

unsafe extern "system" fn console_ctrl_handler(_ctrl_type: u32) -> windows::core::BOOL {
    app::post_shutdown_from_console();
    windows::core::BOOL(1)
}

fn show_startup_error(message: &str) {
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(wide.as_ptr()),
            w!("D2R LRT Tracker"),
            MB_OK | MB_ICONERROR,
        );
    }
}
