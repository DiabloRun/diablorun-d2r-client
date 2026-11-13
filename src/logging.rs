//! Small size-capped file/console logger.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl Level {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Self::Error,
            "warn" | "warning" => Self::Warn,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Info,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

struct Logger {
    file: Option<File>,
    path: PathBuf,
    max_bytes: u64,
    written: u64,
    console: bool,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
static MAX_LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn log_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("logs")
        .join("client.log")
}

pub fn init(
    config_path: &Path,
    level: Level,
    max_size_mb: u64,
    console: bool,
) -> io::Result<PathBuf> {
    MAX_LEVEL.store(level as u8, Ordering::Relaxed);
    let path = log_path(config_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    rotate_if_needed(&path, max_size_mb * 1024 * 1024);

    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);

    let logger = Logger {
        file: Some(file),
        path: path.clone(),
        max_bytes: (max_size_mb * 1024 * 1024).max(1024),
        written,
        console,
    };

    let _ = LOGGER.set(Mutex::new(logger));
    Ok(path)
}

pub fn log(level: Level, message: std::fmt::Arguments<'_>) {
    let max = MAX_LEVEL.load(Ordering::Relaxed);
    if level as u8 > max {
        return;
    }

    let line = format!("[{}] [{}] {}\n", local_timestamp(), level.label(), message);

    if let Some(logger) = LOGGER.get() {
        let mut logger = logger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if logger.console {
            let _ = io::stdout().write_all(line.as_bytes());
            let _ = io::stdout().flush();
        }

        if logger.written + line.len() as u64 > logger.max_bytes {
            rotate(&mut logger);
        }

        if let Some(file) = logger.file.as_mut() {
            let wrote = file.write_all(line.as_bytes()).is_ok() && file.flush().is_ok();
            if wrote {
                logger.written += line.len() as u64;
            }
        }
    } else if level as u8 <= max {
        // Before `init` (or if initialization failed), diagnostics still go to
        // the console so startup errors are visible.
        eprint!("{line}");
    }
}

fn rotate_if_needed(path: &Path, max_bytes: u64) {
    let too_large = fs::metadata(path)
        .map(|metadata| metadata.len() > max_bytes)
        .unwrap_or(false);
    if too_large {
        let rotated = path.with_extension("log.1");
        let _ = fs::remove_file(&rotated);
        let _ = fs::rename(path, rotated);
    }
}

fn rotate(logger: &mut Logger) {
    logger.file = None;
    let rotated = logger.path.with_extension("log.1");
    let _ = fs::remove_file(&rotated);
    let _ = fs::rename(&logger.path, &rotated);
    if let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&logger.path)
    {
        logger.file = Some(file);
        logger.written = 0;
    }
}

#[cfg(windows)]
fn local_timestamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;

    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

#[cfg(not(windows))]
fn local_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Error, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Warn, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Info, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Debug, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Trace, format_args!($($arg)*))
    };
}
