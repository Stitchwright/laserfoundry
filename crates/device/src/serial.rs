//! Real serial-port transport (feature `serial`).

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use crate::Transport;
use crate::linebuf::LineBuffer;

/// Typical GRBL laser baud rate.
pub const DEFAULT_BAUD: u32 = 115_200;

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
    lines: LineBuffer,
}

impl SerialTransport {
    pub fn open(path: &str, baud: u32) -> io::Result<Self> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(10))
            .open()
            .map_err(|e| io::Error::other(format!("{e}{}", open_hint(&e.to_string()))))?;
        Ok(Self { port, lines: LineBuffer::default() })
    }
}

/// What usually fixes a port that will not open, judged from the system's own message: the first things to
/// try on a first connection, since the message alone ("Permission denied") rarely says.
pub fn open_hint(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if m.contains("permission denied") {
        " — on Linux your user must be allowed to use serial ports (add it to the `dialout` or `uucp` group, then sign out and in again); another program may also hold the port"
    } else if m.contains("access is denied") || m.contains("busy") || m.contains("in use") || m.contains("resource") {
        " — another program (a laser program, a terminal, a slicer) may be using the port: close it and try again"
    } else if m.contains("no such file") || m.contains("not found") || m.contains("cannot find") {
        " — the port is gone: check the USB cable and choose the port again"
    } else {
        ""
    }
}

/// Names of serial ports currently present.
pub fn list_ports() -> Vec<String> {
    serialport::available_ports().map(|ports| ports.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

impl Transport for SerialTransport {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.write_all(bytes)?;
        self.port.flush()
    }

    fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        let deadline = Instant::now() + timeout;
        let mut byte = [0u8; 1];
        loop {
            match self.port.read(&mut byte) {
                Ok(1) => {
                    if let Some(line) = self.lines.push(byte[0]) {
                        return Ok(Some(line));
                    }
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_that_will_not_open_says_what_to_try() {
        assert!(open_hint("Permission denied").contains("dialout"));
        assert!(open_hint("Access is denied.").contains("another program"));
        assert!(open_hint("Device or resource busy").contains("another program"));
        assert!(open_hint("No such file or directory").contains("USB cable"));
        assert_eq!(open_hint("something unexpected"), "");
    }
}
