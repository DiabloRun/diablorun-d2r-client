//! TCP transport for LiveSplit's command server (default port 16834).

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::Connection;

pub struct TcpConnection {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    timeout: Duration,
}

impl TcpConnection {
    pub fn connect(host: &str, port: u16, timeout: Duration) -> io::Result<Self> {
        let addresses = (host, port).to_socket_addrs()?;
        let mut last_error = None;

        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(timeout))?;
                    stream.set_write_timeout(Some(timeout))?;
                    stream.set_nodelay(true)?;
                    let reader = BufReader::new(stream.try_clone()?);
                    return Ok(Self {
                        stream,
                        reader,
                        timeout,
                    });
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
}

impl Connection for TcpConnection {
    fn send_command(&mut self, command: &str) -> io::Result<()> {
        let bytes =
            crate::livesplit::protocol::frame_command(crate::config::TransportKind::Tcp, command);
        self.stream.write_all(&bytes)?;
        self.stream.flush()
    }

    fn read_response(&mut self) -> io::Result<String> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "LiveSplit closed the TCP connection",
            ));
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.timeout = timeout;
        self.stream.set_read_timeout(Some(timeout))?;
        self.stream.set_write_timeout(Some(timeout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn exchanges_commands_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let port = listener.local_addr().expect("local addr").port();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));

            let mut line = String::new();
            reader.read_line(&mut line).expect("read line");
            assert_eq!(line, "ping\n");

            let mut writer = stream;
            writer.write_all(b"pong\n").expect("write");
        });

        let mut connection =
            TcpConnection::connect("127.0.0.1", port, Duration::from_secs(2)).expect("connect");
        connection.send_command("ping").expect("send");
        assert_eq!(connection.read_response().expect("read"), "pong");
        server.join().expect("join server");
    }

    #[test]
    fn reports_connection_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let result = TcpConnection::connect("127.0.0.1", port, Duration::from_millis(200));
        assert!(result.is_err());
    }
}
