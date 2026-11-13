//! LiveSplit command protocol helpers.

use crate::config::TransportKind;

/// Commands used by the load remover.
pub const PAUSE_GAME_TIME: &str = "pausegametime";
pub const UNPAUSE_GAME_TIME: &str = "unpausegametime";
pub const GET_CURRENT_TIMER_PHASE: &str = "getcurrenttimerphase";
pub const PING: &str = "ping";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerPhase {
    NotRunning,
    Running,
    Paused,
    Ended,
    Unknown,
}

impl TimerPhase {
    pub fn parse(value: &str) -> Self {
        match normalize_response(value).as_deref() {
            Some("NotRunning") => Self::NotRunning,
            Some("Running") => Self::Running,
            Some("Paused") => Self::Paused,
            Some("Ended") => Self::Ended,
            _ => Self::Unknown,
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRunning => "NotRunning",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Ended => "Ended",
            Self::Unknown => "Unknown",
        }
    }
}

/// Strip transport framing and turn LiveSplit's `-` null response into `None`.
pub fn normalize_response(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|c| c == '\r' || c == '\n').trim();
    if trimmed.is_empty() || trimmed == "-" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Pipe and TCP use newline-delimited commands; WebSocket uses one command per
/// message and must not append a newline.
pub fn frame_command(transport: TransportKind, command: &str) -> Vec<u8> {
    match transport {
        TransportKind::Pipe | TransportKind::Tcp => {
            let mut bytes = Vec::with_capacity(command.len() + 1);
            bytes.extend_from_slice(command.as_bytes());
            bytes.push(b'\n');
            bytes
        }
        TransportKind::WebSocket => command.as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_pipe_and_tcp_with_newline() {
        assert_eq!(
            frame_command(TransportKind::Pipe, "pausegametime"),
            b"pausegametime\n"
        );
        assert_eq!(
            frame_command(TransportKind::Tcp, "pausegametime"),
            b"pausegametime\n"
        );
    }

    #[test]
    fn frames_websocket_without_newline() {
        assert_eq!(
            frame_command(TransportKind::WebSocket, "pausegametime"),
            b"pausegametime"
        );
    }

    #[test]
    fn parses_timer_phases() {
        assert_eq!(TimerPhase::parse("NotRunning"), TimerPhase::NotRunning);
        assert_eq!(TimerPhase::parse("Running\r\n"), TimerPhase::Running);
        assert_eq!(TimerPhase::parse("Paused"), TimerPhase::Paused);
        assert_eq!(TimerPhase::parse("Ended"), TimerPhase::Ended);
        assert_eq!(TimerPhase::parse("-"), TimerPhase::Unknown);
        assert_eq!(TimerPhase::parse("bogus"), TimerPhase::Unknown);
    }

    #[test]
    fn handles_null_and_empty_responses() {
        assert_eq!(normalize_response("-"), None);
        assert_eq!(normalize_response("\r\n"), None);
        assert_eq!(normalize_response("pong\r\n"), Some("pong".into()));
    }
}
