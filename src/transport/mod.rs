//! Blocking LiveSplit transports: named pipe (default), TCP, and WebSocket.

mod pipe;
mod tcp;
mod ws;

use std::io;
use std::time::Duration;

use crate::config::{LiveSplitConfig, TransportKind};

use pipe::PipeConnection;
use tcp::TcpConnection;
use ws::WebSocketConnection;

/// A connected LiveSplit command channel.
pub trait Connection: Send {
    fn send_command(&mut self, command: &str) -> io::Result<()>;
    fn read_response(&mut self) -> io::Result<String>;
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()>;
}

pub fn connect(settings: &LiveSplitConfig) -> io::Result<Box<dyn Connection>> {
    connect_with_timeout(settings, Duration::from_millis(1_500))
}

pub fn connect_with_timeout(
    settings: &LiveSplitConfig,
    timeout: Duration,
) -> io::Result<Box<dyn Connection>> {
    match settings.transport {
        TransportKind::Pipe => Ok(Box::new(PipeConnection::connect(timeout)?)),
        TransportKind::Tcp => Ok(Box::new(TcpConnection::connect(
            &settings.host,
            settings.port.min(65535) as u16,
            timeout,
        )?)),
        TransportKind::WebSocket => Ok(Box::new(WebSocketConnection::connect(
            &settings.host,
            settings.port.min(65535) as u16,
            &settings.websocket_path,
            timeout,
        )?)),
    }
}

/// Connect with the supplied settings, send `ping`, and wait for `pong`.
/// Used by the settings dialog's Test connection button.
pub fn test_connection(settings: &LiveSplitConfig) -> Result<TransportKind, String> {
    let mut connection = connect_with_timeout(settings, Duration::from_millis(1_500))
        .map_err(|_| connection_failure(settings))?;

    connection
        .set_timeout(Duration::from_millis(1_500))
        .map_err(|_| connection_failure(settings))?;
    connection
        .send_command(crate::livesplit::protocol::PING)
        .map_err(|_| connection_failure(settings))?;
    let response = connection
        .read_response()
        .map_err(|_| connection_failure(settings))?;

    if response.trim() == "pong" {
        Ok(settings.transport)
    } else {
        Err(format!("Unexpected LiveSplit ping response: {response}"))
    }
}

fn connection_failure(settings: &LiveSplitConfig) -> String {
    match settings.transport {
        TransportKind::Pipe => "Connection failed: is LiveSplit opened?",
        TransportKind::Tcp => "Connection failed: start LiveSplit's TCP server.",
        TransportKind::WebSocket => "Connection failed: start LiveSplit's WebSocket server.",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_message_matches_the_transport() {
        let mut settings = LiveSplitConfig::default();
        assert_eq!(
            connection_failure(&settings),
            "Connection failed: is LiveSplit opened?"
        );

        settings.transport = TransportKind::Tcp;
        assert_eq!(
            connection_failure(&settings),
            "Connection failed: start LiveSplit's TCP server."
        );

        settings.transport = TransportKind::WebSocket;
        assert_eq!(
            connection_failure(&settings),
            "Connection failed: start LiveSplit's WebSocket server."
        );
    }
}
