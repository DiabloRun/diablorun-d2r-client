//! WebSocket transport for LiveSplit (`ws://host:16834/livesplit`).

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use tungstenite::{client, Message, WebSocket};

use super::Connection;

pub struct WebSocketConnection {
    socket: WebSocket<TcpStream>,
    timeout: Duration,
}

impl WebSocketConnection {
    pub fn connect(host: &str, port: u16, path: &str, timeout: Duration) -> io::Result<Self> {
        let addresses = (host, port).to_socket_addrs()?;
        let mut last_error = None;

        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(timeout))?;
                    stream.set_write_timeout(Some(timeout))?;
                    stream.set_nodelay(true)?;

                    let path = if path.starts_with('/') {
                        path.to_string()
                    } else {
                        format!("/{path}")
                    };
                    let url = format!("ws://{host}:{port}{path}");
                    match client(&url, stream) {
                        Ok((socket, _response)) => {
                            return Ok(Self { socket, timeout });
                        }
                        Err(error) => {
                            last_error =
                                Some(io::Error::new(io::ErrorKind::Other, error.to_string()));
                        }
                    }
                }
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("could not resolve {host}:{port}"),
            )
        }))
    }

    fn apply_timeout(&self, timeout: Duration) -> io::Result<()> {
        self.socket.get_ref().set_read_timeout(Some(timeout))?;
        self.socket.get_ref().set_write_timeout(Some(timeout))
    }
}

impl Connection for WebSocketConnection {
    fn send_command(&mut self, command: &str) -> io::Result<()> {
        self.socket
            .send(Message::text(command))
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
        self.socket
            .flush()
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))
    }

    fn read_response(&mut self) -> io::Result<String> {
        loop {
            let message = self
                .socket
                .read()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;

            match message {
                Message::Text(text) => return Ok(text.to_string()),
                Message::Binary(bytes) => {
                    return Ok(String::from_utf8_lossy(&bytes).to_string());
                }
                Message::Close(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "LiveSplit closed the WebSocket",
                    ));
                }
                Message::Ping(payload) => {
                    self.socket
                        .send(Message::Pong(payload))
                        .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
                }
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.timeout = timeout;
        self.apply_timeout(timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::TcpListener;

    #[test]
    fn exchanges_commands_over_websocket() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("timeout");
            let mut socket = tungstenite::accept(stream).expect("accept websocket");
            let message = socket.read().expect("read");
            assert_eq!(message, Message::text("ping"));
            socket.send(Message::text("pong")).expect("write");
            socket.flush().expect("flush");
        });

        let mut connection =
            WebSocketConnection::connect("127.0.0.1", port, "/livesplit", Duration::from_secs(2))
                .expect("connect");
        connection.send_command("ping").expect("send");
        assert_eq!(connection.read_response().expect("read"), "pong");
        server.join().expect("join");
    }

    #[test]
    fn does_not_append_newlines_to_websocket_messages() {
        // Covered directly by the protocol framing tests as well; this guards
        // against accidental changes in the transport implementation.
        assert_eq!(
            crate::livesplit::protocol::frame_command(
                crate::config::TransportKind::WebSocket,
                "pausegametime"
            ),
            b"pausegametime"
        );
    }
}
