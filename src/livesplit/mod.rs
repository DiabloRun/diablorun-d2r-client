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

        log_debug!("LiveSplit command: {command}");
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

pub fn run(shared: Arc<SharedState>, connector: Arc<Connector>) {
    log_info!("LiveSplit worker started");
    let initial = shared.snapshot();
    let mut controller = Controller::new(initial.settings.livesplit.clone());
    let mut status_connected = false;
    let mut last_revision = initial.revision;
    let mut last_reconnect_generation = initial.reconnect_generation;
    let mut last_connect_error: Option<String> = None;
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
            status_connected = false;
            next_retry = Instant::now();
            next_poll = Instant::now();
        }

        if snapshot.settings.livesplit != *controller.settings() {
            controller.disconnect();
            controller.apply_settings(snapshot.settings.livesplit.clone());
            status_connected = false;
            next_retry = Instant::now();
            next_poll = Instant::now();
        }

        let desired = snapshot.enabled && snapshot.desired_paused;

        if !controller.is_connected() {
            if Instant::now() >= next_retry {
                shared.set_livesplit_status(LiveSplitStatus::Connecting, "");
                match controller.connect(desired, connector.as_ref()) {
                    Ok(()) => {
                        status_connected = true;
                        last_connect_error = None;
                        shared.set_livesplit_status(
                            LiveSplitStatus::Connected,
                            format!(
                                "{} ({})",
                                controller.transport_name(),
                                controller.phase().as_str()
                            ),
                        );
                        log_info!("Connected to LiveSplit via {}", controller.transport_name());
                        next_poll = Instant::now() + PHASE_POLL_INTERVAL;
                    }
                    Err(error) => {
                        controller.disconnect();
                        status_connected = false;
                        let message = error.to_string();
                        if last_connect_error.as_deref() != Some(message.as_str()) {
                            log_warn!("LiveSplit connection failed: {message}");
                            last_connect_error = Some(message.clone());
                        } else {
                            log_debug!("LiveSplit connection still unavailable: {message}");
                        }
                        shared.set_livesplit_status(
                            LiveSplitStatus::Disconnected,
                            format!("{}: {error}", controller.transport_name()),
                        );
                        next_retry = Instant::now()
                            + Duration::from_millis(snapshot.settings.livesplit.reconnect_ms);
                    }
                }
            }
        } else if Instant::now() >= next_poll {
            match controller.tick(desired, true) {
                Ok(()) => {
                    next_poll = Instant::now() + PHASE_POLL_INTERVAL;
                    if !status_connected {
                        status_connected = true;
                    }
                    shared.set_livesplit_status(
                        LiveSplitStatus::Connected,
                        format!(
                            "{} ({})",
                            controller.transport_name(),
                            controller.phase().as_str()
                        ),
                    );
                }
                Err(error) => {
                    controller.disconnect();
                    status_connected = false;
                    last_connect_error = Some(error.to_string());
                    log_warn!("LiveSplit connection lost: {error}");
                    shared.set_livesplit_status(LiveSplitStatus::Disconnected, error.to_string());
                    next_retry = Instant::now()
                        + Duration::from_millis(snapshot.settings.livesplit.reconnect_ms);
                }
            }
        } else if snapshot.revision != last_revision {
            if let Err(error) = controller.send_desired(desired) {
                controller.disconnect();
                status_connected = false;
                last_connect_error = Some(error.to_string());
                log_warn!("LiveSplit command failed: {error}");
                shared.set_livesplit_status(LiveSplitStatus::Disconnected, error.to_string());
                next_retry = Instant::now()
                    + Duration::from_millis(snapshot.settings.livesplit.reconnect_ms);
            }
        }

        let now = Instant::now();
        let next_wake = if controller.is_connected() {
            next_poll
        } else {
            next_retry
        };
        let timeout = next_wake
            .saturating_duration_since(now)
            .min(PHASE_POLL_INTERVAL)
            .max(Duration::from_millis(20));
        last_revision = shared.wait_for_revision(snapshot.revision, timeout);
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
}
