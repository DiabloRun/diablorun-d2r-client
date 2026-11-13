#[macro_use]
mod logging;
mod app;
mod autostart;
mod capture;
mod config;
mod detector;
mod hotkeys;
mod livesplit;
mod replay;
mod settings_dialog;
mod state;
mod transport;
mod tray;

use std::path::PathBuf;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Console::{
    AllocConsole, AttachConsole, GetConsoleWindow, SetConsoleCtrlHandler, ATTACH_PARENT_PROCESS,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

const MUTEX_NAME: PCWSTR = w!("Local\\diablorun-d2r-load-remover");

struct Cli {
    console: bool,
    no_tray: bool,
    log_level: Option<String>,
    save_frames: Option<PathBuf>,
    replay_frames: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        if std::env::args().any(|arg| arg == "--replay-frames") {
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

    if cli.console {
        ensure_console();
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let loaded = config::load_config();
    let config_path = loaded.path.clone();
    let mut config = loaded.config;
    if let Some(level) = cli.log_level.as_deref() {
        config.logging.level = level.to_string();
    }

    let log_path = logging::init(
        &config_path,
        logging::Level::parse(&config.logging.level),
        config.logging.max_size_mb,
        cli.console,
    );
    match &log_path {
        Ok(path) => log_info!("Log file: {}", path.display()),
        Err(error) => eprintln!("Could not open log file: {error}"),
    }

    if let Some(error) = loaded.error {
        log_warn!("{error}");
    }
    if config_path.parent().is_some_and(|parent| !parent.exists()) {
        if let Err(error) = std::fs::create_dir_all(config_path.parent().unwrap()) {
            log_warn!("Could not create config directory: {error}");
        }
    }
    if !config_path.exists() {
        if let Err(error) = config::save_config(&config_path, &config) {
            log_warn!("Could not create config file: {error}");
        }
    }

    // Keep the mutex handle alive for the lifetime of the process.
    let mutex = match unsafe { CreateMutexW(None, true, MUTEX_NAME) } {
        Ok(handle) => handle,
        Err(error) => {
            log_warn!("Could not create single-instance mutex: {error}");
            Default::default()
        }
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

    log_info!(
        "Starting d2r-load-remover {} (transport {}, enabled={})",
        env!("CARGO_PKG_VERSION"),
        config.livesplit.transport,
        config.enabled
    );
    if config.livesplit.precision_correction {
        log_warn!("precisionCorrection is enabled in config but is not applied by this build");
    }

    let options = app::Options {
        no_tray: cli.no_tray,
        save_frames: cli.save_frames,
    };

    let result = app::run(config, config_path, options);
    log_info!("Exiting");
    result
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        console: false,
        no_tray: false,
        log_level: None,
        save_frames: None,
        replay_frames: None,
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--console" => cli.console = true,
            "--no-tray" => cli.no_tray = true,
            "--log-level" => {
                cli.log_level = Some(
                    args.next()
                        .ok_or_else(|| "--log-level requires a value".to_string())?,
                );
            }
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
                println!("d2r-load-remover {}", env!("CARGO_PKG_VERSION"));
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
            "d2r-load-remover {}\n\n",
            "Usage: d2r-load-remover [options]\n\n",
            "Options:\n",
            "  --console              Attach a console for development output\n",
            "  --log-level <level>    error, warn, info, debug, or trace\n",
            "  --save-frames <dir>    Dump raw detector frames for fixture creation\n",
            "  --replay-frames <dir>  Replay a saved frame dump through the detector\n",
            "  --no-tray              Run headless without a tray icon\n",
            "  --help                 Show this help\n",
            "  --version              Show the version"
        ),
        env!("CARGO_PKG_VERSION")
    );
}

fn ensure_console() {
    unsafe {
        if GetConsoleWindow().is_invalid() {
            let attached = AttachConsole(ATTACH_PARENT_PROCESS);
            if attached.is_err() {
                let _ = AllocConsole();
            }
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
            w!("D2R Load Remover"),
            MB_OK | MB_ICONERROR,
        );
    }
}
