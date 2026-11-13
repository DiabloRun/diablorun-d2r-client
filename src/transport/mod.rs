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
    let mut connection =
        connect_with_timeout(settings, Duration::from_millis(1_500)).map_err(|error| {
            format!(
                "Failed to connect via {}: {error}",
                settings.transport.as_str()
            )
        })?;

    connection
        .set_timeout(Duration::from_millis(1_500))
        .map_err(|error| error.to_string())?;
    connection
        .send_command(crate::livesplit::protocol::PING)
        .map_err(|error| format!("Failed to send ping: {error}"))?;
    let response = connection
        .read_response()
        .map_err(|error| format!("Failed to read ping response: {error}"))?;

    if response.trim() == "pong" {
        Ok(settings.transport)
    } else {
        Err(format!("Unexpected ping response: {response}"))
    }
}
