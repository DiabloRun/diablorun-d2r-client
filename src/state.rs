//! Shared state between the UI, capture thread, and LiveSplit worker.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStatus {
    Disabled,
    Searching,
    Capturing,
    Lost,
    Error,
}

impl CaptureStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "off",
            Self::Searching => "waiting for D2R",
            Self::Capturing => "capturing",
            Self::Lost => "capture lost",
            Self::Error => "capture error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveSplitStatus {
    Disconnected,
    Connecting,
    Connected,
}

impl LiveSplitStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
        }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub enabled: bool,
    pub desired_paused: bool,
    pub settings: Config,
    pub revision: u64,
    pub reconnect_generation: u64,
    pub shutdown: bool,
}

struct Inner {
    enabled: bool,
    desired_paused: bool,
    settings: Config,
    revision: u64,
    reconnect_generation: u64,
    shutdown: bool,
    capture_status: CaptureStatus,
    capture_detail: String,
    livesplit_status: LiveSplitStatus,
    livesplit_detail: String,
    hint: Option<String>,
}

pub struct SharedState {
    inner: Mutex<Inner>,
    changed: Condvar,
}

impl SharedState {
    pub fn new(settings: Config) -> Arc<Self> {
        let enabled = settings.enabled;
        Arc::new(Self {
            inner: Mutex::new(Inner {
                enabled,
                desired_paused: false,
                settings,
                revision: 1,
                reconnect_generation: 1,
                shutdown: false,
                capture_status: CaptureStatus::Searching,
                capture_detail: String::new(),
                livesplit_status: LiveSplitStatus::Disconnected,
                livesplit_detail: String::new(),
                hint: None,
            }),
            changed: Condvar::new(),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Snapshot {
            enabled: inner.enabled,
            desired_paused: inner.desired_paused,
            settings: inner.settings.clone(),
            revision: inner.revision,
            reconnect_generation: inner.reconnect_generation,
            shutdown: inner.shutdown,
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .enabled
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.enabled == enabled {
            return;
        }
        inner.enabled = enabled;
        if !enabled {
            inner.desired_paused = false;
        }
        inner.revision = inner.revision.wrapping_add(1);
        drop(inner);
        self.changed.notify_all();
    }

    pub fn toggle_enabled(&self) -> bool {
        let enabled = self.enabled();
        self.set_enabled(!enabled);
        !enabled
    }

    pub fn set_desired_paused(&self, paused: bool) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.shutdown {
            // Never let a late capture transition re-pause game time while the
            // app is shutting down.
            return;
        }
        if inner.desired_paused == paused {
            return;
        }
        inner.desired_paused = paused;
        inner.revision = inner.revision.wrapping_add(1);
        drop(inner);
        self.changed.notify_all();
    }

    pub fn apply_settings(&self, settings: Config) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.enabled = settings.enabled;
        if !settings.enabled {
            inner.desired_paused = false;
        }
        inner.settings = settings;
        inner.revision = inner.revision.wrapping_add(1);
        drop(inner);
        self.changed.notify_all();
    }

    pub fn request_reconnect(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.reconnect_generation = inner.reconnect_generation.wrapping_add(1);
        inner.revision = inner.revision.wrapping_add(1);
        drop(inner);
        self.changed.notify_all();
    }

    pub fn request_shutdown(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.shutdown = true;
        inner.desired_paused = false;
        inner.revision = inner.revision.wrapping_add(1);
        drop(inner);
        self.changed.notify_all();
    }

    pub fn set_capture_status(&self, status: CaptureStatus, detail: impl Into<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.capture_status = status;
        inner.capture_detail = detail.into();
    }

    pub fn set_livesplit_status(&self, status: LiveSplitStatus, detail: impl Into<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.livesplit_status = status;
        inner.livesplit_detail = detail.into();
    }

    pub fn set_hint(&self, hint: Option<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.hint = hint;
    }

    pub fn tooltip(&self) -> String {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut text = format!(
            "D2R: {} | LiveSplit: {}",
            inner.capture_status.label(),
            inner.livesplit_status.label()
        );

        if let Some(hint) = &inner.hint {
            text.push_str(" | ");
            text.push_str(hint);
        } else if !inner.capture_detail.is_empty() {
            text.push_str(" | ");
            text.push_str(&inner.capture_detail);
        }

        if !inner.livesplit_detail.is_empty() {
            text.push_str(" | ");
            text.push_str(&inner.livesplit_detail);
        }

        text
    }

    /// Wait until the revision changes, the timeout expires, or shutdown is
    /// requested. Returns the current revision.
    pub fn wait_for_revision(&self, revision: u64, timeout: Duration) -> u64 {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.revision == revision && !inner.shutdown {
            let (guard, _) = self
                .changed
                .wait_timeout_while(inner, timeout, |inner| {
                    inner.revision == revision && !inner.shutdown
                })
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner = guard;
        }
        inner.revision
    }

    pub fn notify_all(&self) {
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_clears_desired_pause_and_bumps_revision() {
        let shared = SharedState::new(Config::default());
        let before = shared.snapshot().revision;
        shared.set_desired_paused(true);
        assert!(shared.snapshot().desired_paused);

        shared.set_enabled(false);
        let after = shared.snapshot();
        assert!(!after.enabled);
        assert!(!after.desired_paused);
        assert!(after.revision > before);
    }

    #[test]
    fn desired_pause_only_notifies_on_change() {
        let shared = SharedState::new(Config::default());
        let revision = shared.snapshot().revision;
        shared.set_desired_paused(false);
        assert_eq!(shared.snapshot().revision, revision);

        shared.set_desired_paused(true);
        assert_eq!(shared.snapshot().revision, revision + 1);
    }

    #[test]
    fn shutdown_always_sends_unpause_state() {
        let shared = SharedState::new(Config::default());
        shared.set_desired_paused(true);
        shared.request_shutdown();
        let snapshot = shared.snapshot();
        assert!(snapshot.shutdown);
        assert!(!snapshot.desired_paused);
    }

    #[test]
    fn shutdown_ignores_a_late_capture_pause() {
        let shared = SharedState::new(Config::default());
        shared.request_shutdown();
        shared.set_desired_paused(true);
        assert!(!shared.snapshot().desired_paused);
    }

    #[test]
    fn tooltip_reports_both_halves() {
        let shared = SharedState::new(Config::default());
        shared.set_capture_status(CaptureStatus::Capturing, "D2R.exe 1920x1080");
        shared.set_livesplit_status(LiveSplitStatus::Connected, "pipe");
        let tooltip = shared.tooltip();
        assert!(tooltip.contains("D2R: capturing"));
        assert!(tooltip.contains("LiveSplit: connected"));
        assert!(tooltip.contains("pipe"));
    }
}
