//! LiveSplit connection management and the desired-state state machine.

pub mod protocol;

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::TimerPhase;
use protocol::{GET_CURRENT_TIMER_PHASE, PAUSE_GAME_TIME, UNPAUSE_GAME_TIME};

use crate::config::LiveSplitConfig;
use crate::state::{LiveSplitStatus, SharedState};
use crate::transport::Connection;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const PHASE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// While game time is paused for a load, poll more often so a timer started or
/// reset during the load is noticed quickly.
const LOADING_POLL_INTERVAL: Duration = Duration::from_millis(250);

fn poll_interval(desired_paused: bool) -> Duration {
    if desired_paused {
        LOADING_POLL_INTERVAL
    } else {
        PHASE_POLL_INTERVAL
    }
}

/// Owns one transport connection and tracks what was last sent.
pub struct Controller {
    settings: LiveSplitConfig,
    connection: Option<Box<dyn Connection>>,
    last_sent_paused: Option<bool>,
    pause_suppressed: bool,
    phase: TimerPhase,
}

impl Controller {
    pub fn new(settings: LiveSplitConfig) -> Self {
        Self {
            settings,
            connection: None,
            last_sent_paused: None,
            pause_suppressed: false,
            phase: TimerPhase::Unknown,
        }
    }

    pub fn settings(&self) -> &LiveSplitConfig {
        &self.settings
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn phase(&self) -> TimerPhase {
        self.phase
    }

    pub fn transport_name(&self) -> &'static str {
        self.settings.transport.as_str()
    }

    pub fn disconnect(&mut self) {
        self.connection = None;
        self.last_sent_paused = None;
        self.pause_suppressed = false;
    }

    pub fn apply_settings(&mut self, settings: LiveSplitConfig) -> bool {
        if self.settings == settings {
            return false;
        }
        self.disconnect();
        self.settings = settings;
        true
    }

    /// Connect, send the currently desired state once, and query the timer
    /// phase. The caller must call `disconnect` if this returns an error.
    pub fn connect(&mut self, desired_paused: bool, connector: &Connector) -> io::Result<()> {
        let mut connection = connector(&self.settings)?;
        connection.set_timeout(COMMAND_TIMEOUT)?;

        self.connection = Some(connection);
        self.last_sent_paused = None;
        self.pause_suppressed = false;

        self.send_state(desired_paused)?;
        self.phase = self.query_phase()?;
        self.resync_after_phase(desired_paused)?;
        Ok(())
    }

    fn connection(&mut self) -> io::Result<&mut Box<dyn Connection>> {
        self.connection.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "LiveSplit is not connected")
        })
    }

    fn send_command(&mut self, command: &str, expect_response: bool) -> io::Result<Option<String>> {
        if !self.is_connected() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "LiveSplit is not connected",
            ));
        }

        let connection = self.connection()?;
        connection.send_command(command)?;

        if expect_response {
            Ok(Some(connection.read_response()?))
        } else {
            Ok(None)
        }
    }

    fn send_state(&mut self, paused: bool) -> io::Result<()> {
        let command = if paused {
            PAUSE_GAME_TIME
        } else {
            UNPAUSE_GAME_TIME
        };
        self.send_command(command, false)?;
        self.last_sent_paused = Some(paused);
        Ok(())
    }

    /// Send the desired state unless it is already current, or unless a timer
    /// reset temporarily suppresses pausing until the timer is running again.
    pub fn send_desired(&mut self, desired_paused: bool) -> io::Result<()> {
        let effective = desired_paused && !self.pause_suppressed;
        if self.last_sent_paused != Some(effective) {
            self.send_state(effective)?;
        }
        Ok(())
    }

    pub fn query_phase(&mut self) -> io::Result<TimerPhase> {
        let response = self
            .send_command(GET_CURRENT_TIMER_PHASE, true)?
            .unwrap_or_default();
        let phase = TimerPhase::parse(&response);
        self.phase = phase;
        Ok(phase)
    }

    fn resync_after_phase(&mut self, desired_paused: bool) -> io::Result<()> {
        if desired_paused {
            if self.phase.is_running() {
                self.pause_suppressed = false;
            } else if self.last_sent_paused == Some(true) {
                // Do not leave game time paused across a timer reset or while
                // the timer is stopped/paused. The next Running poll will
                // re-send pause if the loading screen is still showing.
                self.send_state(false)?;
                self.pause_suppressed = true;
            }
        } else if self.phase != TimerPhase::Running {
            // Unpausing is safe even when the user paused the timer.
        }

        self.send_desired(desired_paused)
    }

    pub fn tick(&mut self, desired_paused: bool, poll_phase: bool) -> io::Result<()> {
        if poll_phase {
            self.query_phase()?;
            self.resync_after_phase(desired_paused)?;
        } else {
            self.send_desired(desired_paused)?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if self.connection.is_some() {
            let _ = self.send_state(false);
        }
        self.disconnect();
    }
}

/// A function that opens a transport. Production uses `transport::connect`;
/// tests supply an in-process mock.
pub type Connector = dyn Fn(&LiveSplitConfig) -> io::Result<Box<dyn Connection>> + Send + Sync;

fn mark_disconnected(
    shared: &SharedState,
    controller: &mut Controller,
    next_retry: &mut Instant,
    reconnect_ms: u64,
    error: io::Error,
) {
    controller.disconnect();
    shared.set_livesplit_status(LiveSplitStatus::Disconnected, error.to_string());
    *next_retry = Instant::now() + Duration::from_millis(reconnect_ms);
}

pub fn run(shared: Arc<SharedState>, connector: Arc<Connector>) {
    let initial = shared.snapshot();
    let mut controller = Controller::new(initial.settings.livesplit.clone());
    let mut last_reconnect_generation = initial.reconnect_generation;
    let mut next_retry = Instant::now();
    let mut next_poll = Instant::now();

    loop {
        let snapshot = shared.snapshot();
        if snapshot.shutdown {
            controller.shutdown();
            return;
        }

        if snapshot.reconnect_generation != last_reconnect_generation {
            last_reconnect_generation = snapshot.reconnect_generation;
            controller.disconnect();
            shared.drain_pause_transitions();
            next_retry = Instant::now();
            next_poll = Instant::now();
        }

        if snapshot.settings.livesplit != *controller.settings() {
            controller.disconnect();
            controller.apply_settings(snapshot.settings.livesplit.clone());
            shared.drain_pause_transitions();
            next_retry = Instant::now();
            next_poll = Instant::now();
        }

        let mut desired = snapshot.enabled && snapshot.desired_paused;
        let reconnect_ms = snapshot.settings.livesplit.reconnect_ms;

        if !controller.is_connected() {
            // Transitions cannot be applied while disconnected. The latest
            // desired state is sent as soon as the connection is established.
            shared.drain_pause_transitions();

            if Instant::now() >= next_retry {
                shared.set_livesplit_status(LiveSplitStatus::Connecting, "");
                match controller.connect(desired, connector.as_ref()) {
                    Ok(()) => {
                        shared.set_livesplit_status(
                            LiveSplitStatus::Connected,
                            format!(
                                "{} ({})",
                                controller.transport_name(),
                                controller.phase().as_str()
                            ),
                        );
                        next_poll = Instant::now() + poll_interval(desired);
                    }
                    Err(error) => {
                        controller.disconnect();
                        shared.set_livesplit_status(
                            LiveSplitStatus::Disconnected,
                            format!("{}: {error}", controller.transport_name()),
                        );
                        next_retry = Instant::now() + Duration::from_millis(reconnect_ms);
                    }
                }
            }
        } else {
            // Apply every captured transition in order before doing anything
            // that can block on the transport. This is what keeps the pause
            // latency at one capture frame instead of one phase-poll interval.
            let transitions = shared.drain_pause_transitions();
            // Take a fresh snapshot after the drain. The snapshot from the top
            // of the loop can be older than the transition that woke us; using
            // its desired state here would immediately undo that transition.
            let latest = shared.snapshot();
            desired = latest.enabled && latest.desired_paused;

            for transition in transitions {
                if !latest.enabled {
                    break;
                }
                if let Err(error) = controller.send_desired(transition.paused) {
                    mark_disconnected(
                        &shared,
                        &mut controller,
                        &mut next_retry,
                        reconnect_ms,
                        error,
                    );
                    break;
                }
            }

            // Reconcile with the latest desired state. This covers a dropped
            // transition, a settings change, and the first frame after a
            // reconnect.
            if controller.is_connected() {
                if let Err(error) = controller.send_desired(desired) {
                    mark_disconnected(
                        &shared,
                        &mut controller,
                        &mut next_retry,
                        reconnect_ms,
                        error,
                    );
                }
            }

            if controller.is_connected() && Instant::now() >= next_poll {
                match controller.tick(desired, true) {
                    Ok(()) => {
                        next_poll = Instant::now() + poll_interval(desired);
                        shared.set_livesplit_status(
                            LiveSplitStatus::Connected,
                            format!(
                                "{} ({})",
                                controller.transport_name(),
                                controller.phase().as_str()
                            ),
                        );
                    }
                    Err(error) => mark_disconnected(
                        &shared,
                        &mut controller,
                        &mut next_retry,
                        reconnect_ms,
                        error,
                    ),
                }
            }
        }

        let now = Instant::now();
        let interval = poll_interval(desired);
        if controller.is_connected() && next_poll > now + interval {
            // A transition into loading should tighten the cadence immediately
            // instead of waiting for the old one-second poll deadline.
            next_poll = now + interval;
        }
        let next_wake = if controller.is_connected() {
            next_poll
        } else {
            next_retry
        };
        let timeout = next_wake
            .saturating_duration_since(now)
            .min(interval)
            .max(Duration::from_millis(20));
        // Deliberately do not store the returned revision: the wake itself is
        // the signal that the snapshot must be rebuilt and the desired state
        // re-sent.
        let _ = shared.wait_for_revision(snapshot.revision, timeout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TransportKind;

    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct MockConnection {
        commands: Arc<Mutex<Vec<String>>>,
        responses: VecDeque<String>,
        fail_reads: bool,
    }

    impl MockConnection {
        fn new(commands: Arc<Mutex<Vec<String>>>, responses: &[&str]) -> Self {
            Self {
                commands,
                responses: responses.iter().map(|value| value.to_string()).collect(),
                fail_reads: false,
            }
        }
    }

    impl Connection for MockConnection {
        fn send_command(&mut self, command: &str) -> io::Result<()> {
            self.commands
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(command.to_string());
            Ok(())
        }

        fn read_response(&mut self) -> io::Result<String> {
            if self.fail_reads {
                return Err(io::Error::new(io::ErrorKind::Other, "mock read failed"));
            }
            self.responses
                .pop_front()
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no mock response"))
        }

        fn set_timeout(&mut self, _timeout: Duration) -> io::Result<()> {
            Ok(())
        }
    }

    fn attached_controller(commands: Arc<Mutex<Vec<String>>>, responses: &[&str]) -> Controller {
        let mut controller = Controller::new(LiveSplitConfig::default());
        controller.connection = Some(Box::new(MockConnection::new(commands, responses)));
        controller
    }

    #[test]
    fn only_sends_when_desired_state_changes() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut controller = attached_controller(commands.clone(), &[]);

        controller.send_desired(false).expect("unpause");
        controller.send_desired(false).expect("still unpaused");
        controller.send_desired(true).expect("pause");
        controller.send_desired(true).expect("still paused");
        controller.send_desired(false).expect("unpause");

        assert_eq!(
            *commands.lock().unwrap(),
            vec![UNPAUSE_GAME_TIME, PAUSE_GAME_TIME, UNPAUSE_GAME_TIME]
        );
    }

    #[test]
    fn reconnect_resends_the_current_state() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut controller = attached_controller(commands.clone(), &["Running"]);

        controller.last_sent_paused = Some(true);
        controller.pause_suppressed = true;
        controller.disconnect();
        controller.connection = Some(Box::new(MockConnection::new(
            commands.clone(),
            &["Running"],
        )));
        controller.send_state(true).expect("pause");
        controller.query_phase().expect("phase");

        assert_eq!(
            *commands.lock().unwrap(),
            vec![PAUSE_GAME_TIME, GET_CURRENT_TIMER_PHASE]
        );
        assert_eq!(controller.last_sent_paused, Some(true));
    }

    #[test]
    fn timer_reset_unpauses_and_running_repauses_while_loading() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut controller = attached_controller(commands.clone(), &["NotRunning", "Running"]);

        controller.last_sent_paused = Some(true);
        controller.tick(true, true).expect("reset poll");
        assert!(controller.pause_suppressed);

        controller.tick(true, true).expect("running poll");
        assert!(!controller.pause_suppressed);
        assert_eq!(
            *commands.lock().unwrap(),
            vec![
                GET_CURRENT_TIMER_PHASE,
                UNPAUSE_GAME_TIME,
                GET_CURRENT_TIMER_PHASE,
                PAUSE_GAME_TIME
            ]
        );
    }

    #[test]
    fn disabling_sends_unpause() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut controller = attached_controller(commands.clone(), &[]);
        controller.send_state(true).expect("pause");
        controller.send_desired(false).expect("disable");

        assert_eq!(
            *commands.lock().unwrap(),
            vec![PAUSE_GAME_TIME, UNPAUSE_GAME_TIME]
        );
    }

    #[test]
    fn shutdown_unpauses_and_drops_connection() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut controller = attached_controller(commands.clone(), &[]);
        controller.send_state(true).expect("pause");
        controller.shutdown();

        assert_eq!(
            *commands.lock().unwrap(),
            vec![PAUSE_GAME_TIME, UNPAUSE_GAME_TIME]
        );
        assert!(!controller.is_connected());
    }

    #[test]
    fn apply_settings_disconnects_when_transport_changes() {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut controller = attached_controller(commands, &[]);
        let mut settings = LiveSplitConfig::default();
        settings.transport = TransportKind::Tcp;

        assert!(controller.apply_settings(settings.clone()));
        assert!(!controller.is_connected());
        assert_eq!(controller.settings().transport, TransportKind::Tcp);
        assert!(!controller.apply_settings(settings));
    }

    struct RecordingConnection {
        commands: Arc<Mutex<Vec<String>>>,
    }

    impl Connection for RecordingConnection {
        fn send_command(&mut self, command: &str) -> io::Result<()> {
            self.commands
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(command.to_string());
            Ok(())
        }

        fn read_response(&mut self) -> io::Result<String> {
            Ok("Running".into())
        }

        fn set_timeout(&mut self, _timeout: Duration) -> io::Result<()> {
            Ok(())
        }
    }

    fn wait_for_commands(
        commands: &Arc<Mutex<Vec<String>>>,
        timeout: Duration,
        predicate: impl Fn(&[String]) -> bool,
    ) -> Vec<String> {
        let deadline = Instant::now() + timeout;
        loop {
            let seen = commands
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if predicate(&seen) {
                return seen;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for LiveSplit commands; saw {seen:?}");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn state_changes_are_applied_without_waiting_for_the_phase_poll() {
        let shared = SharedState::new(crate::config::Config::default());
        let commands = Arc::new(Mutex::new(Vec::new()));
        let connector_commands = commands.clone();
        let connector: Arc<Connector> = Arc::new(move |_settings| {
            Ok(Box::new(RecordingConnection {
                commands: connector_commands.clone(),
            }))
        });

        let worker_shared = shared.clone();
        let worker = std::thread::spawn(move || run(worker_shared, connector));

        wait_for_commands(&commands, Duration::from_secs(2), |seen| {
            seen.iter()
                .any(|command| command == GET_CURRENT_TIMER_PHASE)
        });
        commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();

        shared.set_desired_paused_at(true, 10);
        wait_for_commands(&commands, Duration::from_millis(400), |seen| {
            seen.iter().any(|command| command == PAUSE_GAME_TIME)
        });

        commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        shared.set_desired_paused_at(false, 20);
        wait_for_commands(&commands, Duration::from_millis(400), |seen| {
            seen.iter().any(|command| command == UNPAUSE_GAME_TIME)
        });

        commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        shared.set_desired_paused_at(true, 30);
        shared.set_desired_paused_at(false, 40);
        let seen = wait_for_commands(&commands, Duration::from_millis(400), |seen| {
            seen.iter().any(|command| command == PAUSE_GAME_TIME)
                && seen.iter().any(|command| command == UNPAUSE_GAME_TIME)
        });
        let pause = seen
            .iter()
            .position(|command| command == PAUSE_GAME_TIME)
            .expect("pause command");
        let unpause = seen
            .iter()
            .position(|command| command == UNPAUSE_GAME_TIME)
            .expect("unpause command");
        assert!(
            pause < unpause,
            "pause must be sent before resume: {seen:?}"
        );

        shared.request_shutdown();
        worker.join().expect("worker exit");
    }
}
