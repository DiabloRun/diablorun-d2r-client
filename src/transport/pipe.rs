//! Named-pipe transport using overlapped Win32 I/O so reads and writes have a
//! bounded timeout and cannot wedge the LiveSplit worker forever.

use std::io;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_IO_PENDING, ERROR_PIPE_BUSY, GENERIC_READ,
    GENERIC_WRITE, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::Connection;

const PIPE_NAME: &str = r"\\.\pipe\LiveSplit";

pub struct PipeConnection {
    handle: isize,
    event: isize,
    timeout: Duration,
    read_buffer: Vec<u8>,
}

// The raw handles are only used from the thread that owns the connection.
unsafe impl Send for PipeConnection {}

impl PipeConnection {
    pub fn connect(timeout: Duration) -> io::Result<Self> {
        Self::connect_to_name(PIPE_NAME, timeout)
    }

    fn connect_to_name(name: &str, timeout: Duration) -> io::Result<Self> {
        let wide = to_wide(name);
        let name = PCWSTR(wide.as_ptr());
        let deadline = Instant::now() + timeout;

        loop {
            match open_pipe(name) {
                Ok(handle) => {
                    let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
                        .map_err(|error| {
                            let _ = unsafe { CloseHandle(handle) };
                            win_error(error)
                        })?;

                    return Ok(Self {
                        handle: handle.0 as isize,
                        event: event.0 as isize,
                        timeout,
                        read_buffer: Vec::with_capacity(1024),
                    });
                }
                Err(error) if is_win32(error.code().0, ERROR_PIPE_BUSY.0) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Timed out waiting for the LiveSplit named pipe",
                        ));
                    }

                    let wait_ms = remaining.as_millis().min(u32::MAX as u128) as u32;
                    let signaled = unsafe { WaitNamedPipeW(name, wait_ms) };
                    if !signaled.as_bool() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Timed out waiting for the LiveSplit named pipe",
                        ));
                    }
                }
                Err(error) if is_win32(error.code().0, ERROR_ACCESS_DENIED.0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Access denied opening LiveSplit's named pipe. If LiveSplit is \
                         running elevated, run this app elevated as well, or restart \
                         LiveSplit without elevation.",
                    ));
                }
                Err(error) => return Err(win_error(error)),
            }
        }
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.handle as _)
    }

    fn event(&self) -> HANDLE {
        HANDLE(self.event as _)
    }

    fn timeout_ms(&self) -> u32 {
        self.timeout.as_millis().min(u32::MAX as u128) as u32
    }

    fn overlapped_io<F>(&mut self, operation: F) -> io::Result<u32>
    where
        F: FnOnce(HANDLE, *mut OVERLAPPED, *mut u32) -> windows::core::Result<()>,
    {
        unsafe {
            ResetEvent(self.event()).map_err(win_error)?;
        }

        let mut overlapped = OVERLAPPED::default();
        overlapped.hEvent = self.event();

        let mut transferred = 0u32;
        match operation(self.handle(), &mut overlapped, &mut transferred) {
            Ok(()) => Ok(transferred),
            Err(error) if is_win32(error.code().0, ERROR_IO_PENDING.0) => {
                let wait = unsafe { WaitForSingleObject(self.event(), self.timeout_ms()) };
                if wait == WAIT_OBJECT_0 {
                    unsafe {
                        GetOverlappedResult(self.handle(), &overlapped, &mut transferred, false)
                    }
                    .map_err(win_error)?;
                    Ok(transferred)
                } else {
                    unsafe {
                        let _ = CancelIoEx(self.handle(), Some(&overlapped));
                        // CancelIoEx is asynchronous and `overlapped` lives on
                        // this stack frame. Wait for the completion event so
                        // Windows cannot write into freed stack memory after
                        // we return.
                        let _ = WaitForSingleObject(self.event(), u32::MAX);
                        let mut ignored = 0u32;
                        let _ =
                            GetOverlappedResult(self.handle(), &overlapped, &mut ignored, false);
                    }
                    let message = if wait == WAIT_TIMEOUT {
                        "Timed out waiting for the LiveSplit named pipe"
                    } else {
                        "Named-pipe operation failed"
                    };
                    Err(io::Error::new(io::ErrorKind::TimedOut, message))
                }
            }
            Err(error) => Err(win_error(error)),
        }
    }
}

impl Connection for PipeConnection {
    fn send_command(&mut self, command: &str) -> io::Result<()> {
        let bytes =
            crate::livesplit::protocol::frame_command(crate::config::TransportKind::Pipe, command);
        let mut offset = 0;

        while offset < bytes.len() {
            let chunk = &bytes[offset..];
            let written = self.overlapped_io(|handle, overlapped, transferred| unsafe {
                WriteFile(handle, Some(chunk), Some(transferred), Some(overlapped))
            })?;

            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "LiveSplit pipe accepted zero bytes",
                ));
            }
            offset += written as usize;
        }

        Ok(())
    }

    fn read_response(&mut self) -> io::Result<String> {
        loop {
            if let Some(position) = self.read_buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.read_buffer.drain(..=position).collect();
                let text = String::from_utf8_lossy(&line);
                return Ok(text.trim_end_matches(['\r', '\n']).to_string());
            }

            let mut chunk = [0u8; 4096];
            let read = self.overlapped_io(|handle, overlapped, transferred| unsafe {
                ReadFile(
                    handle,
                    Some(&mut chunk),
                    Some(transferred),
                    Some(overlapped),
                )
            })?;

            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "LiveSplit closed the named pipe",
                ));
            }

            self.read_buffer.extend_from_slice(&chunk[..read as usize]);
        }
    }

    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(HANDLE(self.handle as _));
            let _ = CloseHandle(HANDLE(self.event as _));
        }
    }
}

fn open_pipe(name: PCWSTR) -> windows::core::Result<HANDLE> {
    unsafe {
        CreateFileW(
            name,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            None,
        )
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn is_win32(hresult: i32, code: u32) -> bool {
    (hresult as u32 & 0xFFFF) == (code & 0xFFFF)
}

fn win_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32 & 0xFFFF;
    io::Error::from_raw_os_error(code as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    struct PipeServer {
        handle: isize,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for PipeServer {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            unsafe {
                let handle = HANDLE(self.handle as _);
                let _ = DisconnectNamedPipe(handle);
                let _ = CloseHandle(handle);
            }
        }
    }

    fn start_server(name: &str, response: &'static str) -> PipeServer {
        let wide = to_wide(name);
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!handle.is_invalid(), "create named pipe");

        let thread_handle = handle.0 as isize;
        let thread = std::thread::spawn(move || {
            let thread_handle = HANDLE(thread_handle as _);
            unsafe {
                let _ = ConnectNamedPipe(thread_handle, None);
            }

            let mut command = Vec::new();
            let mut chunk = [0u8; 256];
            loop {
                let mut read = 0u32;
                let result =
                    unsafe { ReadFile(thread_handle, Some(&mut chunk), Some(&mut read), None) };
                if result.is_err() || read == 0 {
                    break;
                }
                command.extend_from_slice(&chunk[..read as usize]);
                if command.contains(&b'\n') {
                    break;
                }
            }

            assert_eq!(command, b"ping\n");

            let mut written = 0u32;
            unsafe {
                let _ = WriteFile(
                    thread_handle,
                    Some(response.as_bytes()),
                    Some(&mut written),
                    None,
                );
            }
        });

        PipeServer {
            handle: handle.0 as isize,
            thread: Some(thread),
        }
    }

    #[test]
    fn exchanges_commands_over_a_named_pipe() {
        let name = format!(r"\\.\pipe\d2r-lrt-tracker-test-{}", std::process::id());
        let _server = start_server(&name, "pong\n");

        let mut connection =
            PipeConnection::connect_to_name(&name, Duration::from_secs(2)).expect("connect pipe");
        connection
            .set_timeout(Duration::from_secs(2))
            .expect("set timeout");
        connection.send_command("ping").expect("send");
        assert_eq!(connection.read_response().expect("read"), "pong");
    }
}
