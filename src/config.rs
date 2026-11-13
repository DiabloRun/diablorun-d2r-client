//! Portable configuration loading, merging, validation, and saving.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::hotkeys;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    Pipe,
    Tcp,
    WebSocket,
}

impl Default for TransportKind {
    fn default() -> Self {
        Self::Pipe
    }
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Tcp => "tcp",
            Self::WebSocket => "websocket",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pipe" | "named-pipe" | "namedpipe" => Some(Self::Pipe),
            "tcp" => Some(Self::Tcp),
            "websocket" | "ws" => Some(Self::WebSocket),
            _ => None,
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TransportKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TransportKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(Self::parse(&value).unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub capture: CaptureConfig,
    pub detection: DetectionConfig,
    pub livesplit: LiveSplitConfig,
    pub hotkeys: HotkeyConfig,
    pub logging: LoggingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            capture: CaptureConfig::default(),
            detection: DetectionConfig::default(),
            livesplit: LiveSplitConfig::default(),
            hotkeys: HotkeyConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    pub fps: u32,
    pub process: String,
    pub window_title: String,
    pub min_width: u32,
    pub min_height: u32,
    pub pause_on_capture_lost: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            fps: 60,
            process: "D2R.exe".into(),
            window_title: "Diablo II: Resurrected".into(),
            min_width: 160,
            min_height: 160,
            pause_on_capture_lost: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectionConfig {
    pub confirm_frames: u32,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self { confirm_frames: 1 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveSplitConfig {
    pub transport: TransportKind,
    pub host: String,
    pub port: u32,
    pub websocket_path: String,
    pub reconnect_ms: u64,
    pub precision_correction: bool,
}

impl Default for LiveSplitConfig {
    fn default() -> Self {
        Self {
            transport: TransportKind::Pipe,
            host: "127.0.0.1".into(),
            port: 16834,
            websocket_path: "/livesplit".into(),
            reconnect_ms: 2000,
            precision_correction: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub toggle: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Alt+L".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub max_size_mb: u64,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            max_size_mb: 2,
        }
    }
}

pub struct LoadedConfig {
    pub config: Config,
    pub path: PathBuf,
    pub error: Option<String>,
}

/// Resolve the configuration directory. Portable mode wins only when a
/// `config.json` already exists next to the executable.
pub fn resolve_config_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));

    resolve_config_path_from(&exe_dir, std::env::var_os("APPDATA"))
}

pub fn resolve_config_path_from(exe_dir: &Path, appdata: Option<std::ffi::OsString>) -> PathBuf {
    let portable = exe_dir.join("config.json");
    if portable.is_file() {
        return portable;
    }

    roaming_config_dir_from(appdata).join("config.json")
}

fn roaming_config_dir_from(appdata: Option<std::ffi::OsString>) -> PathBuf {
    if let Some(appdata) = appdata {
        PathBuf::from(appdata).join("diablorun-d2r-load-remover")
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

pub fn load_config() -> LoadedConfig {
    load_config_from(resolve_config_path())
}

pub fn load_config_from(path: PathBuf) -> LoadedConfig {
    match fs::read_to_string(&path) {
        Ok(raw) => match parse_config(&raw) {
            Ok(config) => LoadedConfig {
                config,
                path,
                error: None,
            },
            Err(error) => LoadedConfig {
                config: Config::default(),
                path,
                error: Some(format!("Could not parse config: {error}. Using defaults.")),
            },
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => LoadedConfig {
            config: Config::default(),
            path,
            error: None,
        },
        Err(error) => LoadedConfig {
            config: Config::default(),
            path,
            error: Some(format!("Could not read config: {error}. Using defaults.")),
        },
    }
}

pub fn parse_config(raw: &str) -> Result<Config, String> {
    let incoming: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    let defaults = serde_json::to_value(Config::default()).map_err(|error| error.to_string())?;
    let merged = merge_with_defaults(&defaults, &incoming);
    let mut config: Config = serde_json::from_value(merged).map_err(|error| error.to_string())?;
    config.normalize();
    Ok(config)
}

/// Recursively merge user values over defaults. Unknown keys are dropped and
/// values of the wrong JSON type fall back to the default, matching the
/// existing TypeScript client's behavior.
fn merge_with_defaults(defaults: &Value, incoming: &Value) -> Value {
    match defaults {
        Value::Object(default_map) => {
            let incoming_map = incoming.as_object();
            let mut result = serde_json::Map::with_capacity(default_map.len());
            for (key, default_value) in default_map {
                let value = incoming_map
                    .and_then(|map| map.get(key))
                    .map(|value| merge_with_defaults(default_value, value))
                    .unwrap_or_else(|| default_value.clone());
                result.insert(key.clone(), value);
            }
            Value::Object(result)
        }
        Value::String(_) => match incoming {
            Value::String(_) => incoming.clone(),
            _ => defaults.clone(),
        },
        Value::Number(_) => match incoming {
            Value::Number(_) => incoming.clone(),
            _ => defaults.clone(),
        },
        Value::Bool(_) => match incoming {
            Value::Bool(_) => incoming.clone(),
            _ => defaults.clone(),
        },
        Value::Array(_) => match incoming {
            Value::Array(_) => incoming.clone(),
            _ => defaults.clone(),
        },
        Value::Null => incoming.clone(),
    }
}

impl Config {
    pub fn normalize(&mut self) {
        let defaults = Self::default();

        self.capture.fps = clamp(self.capture.fps, 1, 240);
        self.capture.min_width = clamp(self.capture.min_width, 1, 16384);
        self.capture.min_height = clamp(self.capture.min_height, 1, 16384);
        if self.capture.process.trim().is_empty() {
            self.capture.process = defaults.capture.process;
        }
        if self.capture.window_title.trim().is_empty() {
            self.capture.window_title = defaults.capture.window_title;
        }

        self.detection.confirm_frames = clamp(self.detection.confirm_frames, 1, 10);

        if self.livesplit.host.trim().is_empty() {
            self.livesplit.host = defaults.livesplit.host;
        }
        if self.livesplit.port == 0 {
            self.livesplit.port = defaults.livesplit.port;
        }
        self.livesplit.port = clamp(self.livesplit.port, 1, 65535);
        if self.livesplit.websocket_path.trim().is_empty() {
            self.livesplit.websocket_path = defaults.livesplit.websocket_path;
        } else if !self.livesplit.websocket_path.starts_with('/') {
            self.livesplit.websocket_path = format!("/{}", self.livesplit.websocket_path.trim());
        }
        self.livesplit.reconnect_ms = clamp(self.livesplit.reconnect_ms, 100, 60_000);

        if !self.hotkeys.toggle.trim().is_empty()
            && hotkeys::parse_hotkey(&self.hotkeys.toggle).is_err()
        {
            self.hotkeys.toggle = defaults.hotkeys.toggle;
        } else {
            self.hotkeys.toggle = self.hotkeys.toggle.trim().to_string();
        }

        self.logging.level = normalize_log_level(&self.logging.level);
        self.logging.max_size_mb = clamp(self.logging.max_size_mb, 1, 100);
    }

    /// Validate settings entered in the dialog. `normalize` is deliberately not
    /// called here so the user can see what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        match self.livesplit.transport {
            TransportKind::Tcp | TransportKind::WebSocket => {
                if self.livesplit.host.trim().is_empty() {
                    return Err("Host is required for TCP and WebSocket.".into());
                }
            }
            TransportKind::Pipe => {}
        }

        if self.livesplit.port == 0 || self.livesplit.port > 65535 {
            return Err("Port must be between 1 and 65535.".into());
        }
        if self.livesplit.reconnect_ms < 100 || self.livesplit.reconnect_ms > 60_000 {
            return Err("Reconnect must be between 100 and 60000 ms.".into());
        }
        if self.livesplit.transport == TransportKind::WebSocket
            && !self.livesplit.websocket_path.starts_with('/')
        {
            return Err("WebSocket path must start with '/'.".into());
        }
        if self.capture.fps < 1 || self.capture.fps > 240 {
            return Err("Capture FPS must be between 1 and 240.".into());
        }
        if self.capture.min_width == 0 || self.capture.min_height == 0 {
            return Err("Minimum window size must be positive.".into());
        }
        if self.capture.process.trim().is_empty() {
            return Err("Process name is required.".into());
        }
        if self.capture.window_title.trim().is_empty() {
            return Err("Window title is required.".into());
        }
        if self.detection.confirm_frames < 1 || self.detection.confirm_frames > 10 {
            return Err("Confirm frames must be between 1 and 10.".into());
        }
        if !self.hotkeys.toggle.trim().is_empty() {
            hotkeys::parse_hotkey(&self.hotkeys.toggle)
                .map_err(|error| format!("Hotkey: {error}"))?;
        }
        if !matches!(
            self.logging.level.to_ascii_lowercase().as_str(),
            "error" | "warn" | "info" | "debug" | "trace"
        ) {
            return Err("Log level must be error, warn, info, debug, or trace.".into());
        }
        if self.logging.max_size_mb == 0 {
            return Err("Maximum log size must be positive.".into());
        }

        Ok(())
    }
}

fn normalize_log_level(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "error" | "warn" | "info" | "debug" | "trace" => normalized,
        _ => "info".into(),
    }
}

fn clamp<T: PartialOrd + Copy>(value: T, min: T, max: T) -> T {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Save config atomically (write temporary file, then rename).
pub fn save_config(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut json = serde_json::to_string_pretty(config)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
    json.push('\n');

    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, json)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_partial_config_with_defaults() {
        let config = parse_config(
            r#"{
                "enabled": false,
                "capture": { "fps": 30 },
                "livesplit": { "transport": "tcp", "host": "localhost" },
                "unknown": { "ignored": true }
            }"#,
        )
        .expect("parse config");

        assert!(!config.enabled);
        assert_eq!(config.capture.fps, 30);
        assert_eq!(config.capture.process, "D2R.exe");
        assert_eq!(config.livesplit.transport, TransportKind::Tcp);
        assert_eq!(config.livesplit.host, "localhost");
        assert_eq!(config.livesplit.port, 16834);
    }

    #[test]
    fn falls_back_for_wrong_types() {
        let config = parse_config(
            r#"{
                "enabled": "yes",
                "capture": { "fps": "fast", "min_width": null },
                "livesplit": { "port": 99999, "transport": "carrier-pigeon" }
            }"#,
        )
        .expect("parse config");

        assert!(config.enabled);
        assert_eq!(config.capture.fps, 60);
        assert_eq!(config.capture.min_width, 160);
        assert_eq!(config.livesplit.port, 65535);
        assert_eq!(config.livesplit.transport, TransportKind::Pipe);
    }

    #[test]
    fn clamps_out_of_range_values() {
        let mut config = Config::default();
        config.capture.fps = 0;
        config.capture.min_width = 0;
        config.detection.confirm_frames = 99;
        config.livesplit.reconnect_ms = 1;
        config.logging.max_size_mb = 0;
        config.hotkeys.toggle = "Ctrl+Nope".into();
        config.normalize();

        assert_eq!(config.capture.fps, 1);
        assert_eq!(config.capture.min_width, 1);
        assert_eq!(config.detection.confirm_frames, 10);
        assert_eq!(config.livesplit.reconnect_ms, 100);
        assert_eq!(config.logging.max_size_mb, 1);
        assert_eq!(config.hotkeys.toggle, "Ctrl+Alt+L");
    }

    #[test]
    fn adds_leading_slash_to_websocket_path() {
        let mut config = Config::default();
        config.livesplit.websocket_path = "livesplit".into();
        config.normalize();
        assert_eq!(config.livesplit.websocket_path, "/livesplit");
    }

    #[test]
    fn validates_dialog_values() {
        let mut config = Config::default();
        config.livesplit.transport = TransportKind::Tcp;
        config.livesplit.host.clear();
        assert!(config.validate().is_err());

        config.livesplit.host = "127.0.0.1".into();
        config.livesplit.port = 0;
        assert!(config.validate().is_err());

        config.livesplit.port = 16834;
        config.livesplit.transport = TransportKind::WebSocket;
        config.livesplit.websocket_path = "livesplit".into();
        assert!(config.validate().is_err());

        config.livesplit.websocket_path = "/livesplit".into();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn saves_and_loads_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "d2r-load-remover-config-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("config.json");

        let mut config = Config::default();
        config.capture.fps = 45;
        config.livesplit.transport = TransportKind::Tcp;
        save_config(&path, &config).expect("save config");

        let loaded = load_config_from(path.clone());
        assert!(loaded.error.is_none());
        assert_eq!(loaded.config, config);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn portable_config_wins_when_present() {
        let root = std::env::temp_dir().join(format!(
            "d2r-load-remover-portable-test-{}",
            std::process::id()
        ));
        let exe_dir = root.join("app");
        let appdata = root.join("roaming");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&exe_dir).expect("create app dir");
        fs::create_dir_all(&appdata).expect("create appdata dir");

        let fallback = resolve_config_path_from(&exe_dir, Some(appdata.clone().into_os_string()));
        assert_eq!(
            fallback,
            appdata
                .join("diablorun-d2r-load-remover")
                .join("config.json")
        );

        fs::write(exe_dir.join("config.json"), "{}").expect("write portable config");
        let portable = resolve_config_path_from(&exe_dir, Some(appdata.into_os_string()));
        assert_eq!(portable, exe_dir.join("config.json"));

        let _ = fs::remove_dir_all(&root);
    }
}
