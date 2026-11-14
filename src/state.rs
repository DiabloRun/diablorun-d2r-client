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
    Off,
    Searching,
    Capturing,
    Lost,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveSplitStatus {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone)]
pub struct Snapshot {
    pub desired_paused: bool,
    pub settings: Arc<Config>,
    pub revision: u64,
    pub reconnect_generation: u64,
    pub shutdown: bool,
}

#[derive(Clone)]
pub struct StatusSnapshot {
    pub capture_status: CaptureStatus,
    pub capture_detail: String,
    pub livesplit_status: LiveSplitStatus,
}

struct Inner {
    desired_paused: bool,
    pause_transitions: VecDeque<PauseTransition>,
    settings: Arc<Config>,
    revision: u64,
    reconnect_generation: u64,
    shutdown: bool,
    capture_status: CaptureStatus,
    capture_detail: String,
    livesplit_status: LiveSplitStatus,
}

pub struct SharedState {
    inner: Mutex<Inner>,
    changed: Condvar,
}

impl SharedState {
    pub fn new(settings: Config) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                desired_paused: false,
                pause_transitions: VecDeque::new(),
                settings: Arc::new(settings),
                revision: 1,
                reconnect_generation: 1,
                shutdown: false,
                capture_status: CaptureStatus::Searching,
                capture_detail: String::new(),
                livesplit_status: LiveSplitStatus::Disconnected,
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
            desired_paused: inner.desired_paused,
            settings: Arc::clone(&inner.settings),
            revision: inner.revision,
            reconnect_generation: inner.reconnect_generation,
            shutdown: inner.shutdown,
        }
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
        // A settings change (re)connects the worker, which sends the current
        // desired state. Old transitions must not be replayed afterwards.
        inner.pause_transitions.clear();
        inner.settings = Arc::new(settings);
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

    pub fn set_livesplit_status(&self, status: LiveSplitStatus) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.livesplit_status = status;
    }

    pub fn status(&self) -> StatusSnapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        StatusSnapshot {
            capture_status: inner.capture_status,
            capture_detail: inner.capture_detail.clone(),
            livesplit_status: inner.livesplit_status,
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn status_reports_both_halves() {
        let shared = SharedState::new(Config::default());
        shared.set_capture_status(
            CaptureStatus::Capturing,
            "Diablo II: Resurrected (1920x1080)",
        );
        shared.set_livesplit_status(LiveSplitStatus::Connected);

        let status = shared.status();
        assert_eq!(status.capture_status, CaptureStatus::Capturing);
        assert_eq!(status.capture_detail, "Diablo II: Resurrected (1920x1080)");
        assert_eq!(status.livesplit_status, LiveSplitStatus::Connected);
    }
}
