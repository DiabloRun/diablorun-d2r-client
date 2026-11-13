//! Shared state between the UI, capture thread, and LiveSplit worker.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::config::Config;

/// A capture-side pause/unpause transition. The frame timestamp is the WGC
/// presentation timestamp (100 ns units) and is kept so the event can be
/// correlated with the frame that caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PauseTransition {
    pub paused: bool,
    pub timestamp: i64,
}

/// Transitions are drained by the LiveSplit worker immediately. The cap only
/// protects against unbounded growth if that worker stalls for a long time.
const MAX_PAUSE_TRANSITIONS: usize = 64;

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
    pause_transitions: VecDeque<PauseTransition>,
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
                pause_transitions: VecDeque::new(),
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
            // Do not replay transitions recorded before the user disabled load
            // removal when it is enabled again.
            inner.pause_transitions.clear();
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
        self.set_desired_paused_at(paused, 0);
    }

    /// Record a pause/unpause transition. Every change is journalled so the
    /// LiveSplit worker can still apply a quick pause+resume pair that happens
    /// between two of its wakeups.
    pub fn set_desired_paused_at(&self, paused: bool, timestamp: i64) {
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
        if inner.pause_transitions.len() >= MAX_PAUSE_TRANSITIONS {
            inner.pause_transitions.pop_front();
        }
        inner
            .pause_transitions
            .push_back(PauseTransition { paused, timestamp });
        inner.revision = inner.revision.wrapping_add(1);
        drop(inner);
        self.changed.notify_all();
    }

    /// Take all transitions recorded since the last drain. The worker sends
    /// them in order; the latest `desired_paused` value remains authoritative.
    pub fn drain_pause_transitions(&self) -> Vec<PauseTransition> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.pause_transitions.drain(..).collect()
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
        // A settings change (re)connects the worker, which sends the current
        // desired state. Old transitions must not be replayed afterwards.
        inner.pause_transitions.clear();
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
        // The reconnect sends the latest desired state; drop transitions that
        // belonged to the old connection.
        inner.pause_transitions.clear();
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
        inner.pause_transitions.clear();
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
    fn pause_transitions_are_journalled_in_order() {
        let shared = SharedState::new(Config::default());
        shared.set_desired_paused_at(true, 10);
        shared.set_desired_paused_at(true, 11);
        shared.set_desired_paused_at(false, 20);

        assert_eq!(
            shared.drain_pause_transitions(),
            vec![
                PauseTransition {
                    paused: true,
                    timestamp: 10
                },
                PauseTransition {
                    paused: false,
                    timestamp: 20
                }
            ]
        );
        assert!(shared.drain_pause_transitions().is_empty());
    }

    #[test]
    fn disabling_discards_pending_pause_transitions() {
        let shared = SharedState::new(Config::default());
        shared.set_desired_paused_at(true, 10);
        shared.set_enabled(false);
        shared.set_enabled(true);

        assert!(shared.drain_pause_transitions().is_empty());
        assert!(!shared.snapshot().desired_paused);
    }

    #[test]
    fn shutdown_discards_pending_pause_transitions() {
        let shared = SharedState::new(Config::default());
        shared.set_desired_paused_at(true, 10);
        shared.request_shutdown();

        assert!(shared.drain_pause_transitions().is_empty());
        assert!(!shared.snapshot().desired_paused);
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
