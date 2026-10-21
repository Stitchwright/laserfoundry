//! Talking to GRBL controllers: a transport abstraction, a character-counting streamer,
//! status parsing, and a simulated controller for tests and the in-app simulator.
//!
//! No UI and no OS serial code lives here; real serial ports implement [`Transport`] elsewhere
//! (see the `serial` feature).

mod codes;
pub mod linebuf;
mod link;
pub mod settings;
mod sim;
mod sim_marlin;
mod status;
mod stream;

#[cfg(feature = "serial")]
pub mod serial;

pub use codes::{alarm_hint, alarm_text, error_hint, error_text};
pub use link::{CONSOLE_LIMIT, JobState, Link, LinkError, MAX_FEED, MAX_JOG_MM, MAX_TRAVEL_MM, Snapshot, jog_line, jog_to_line};
pub use sim::SimulatedGrbl;
pub use sim_marlin::SimulatedMarlin;
pub use status::{MachineState, Status, parse_marlin_position, parse_status};
pub use stream::{Progress, RESTART_BACKUP_LINES, StreamOptions, StreamReport, restart_line, stream_program};

use std::io;
use std::time::Duration;

use thiserror::Error;

/// GRBL's serial RX buffer, in bytes.
pub const GRBL_RX_BUFFER: usize = 128;

/// Which firmware family a [`Link`] talks to. Smoothieware speaks GRBL's protocol (single-byte real-time commands, `?` status
/// reports, `ok`/`error:` replies), so it uses [`Protocol::Grbl`]; Marlin does not, and needs [`Protocol::Marlin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    #[default]
    Grbl,
    /// Marlin: one command per line, each answered `ok` once it is queued; no real-time commands, no `?` reports (position is asked
    /// for with `M114`), `M410` stops motion and `M5` switches the laser off. Written from Marlin's public G-code index and untested
    /// on a machine.
    Marlin,
}

/// What a Marlin link sends to stop a job: stop all motion and clear the planner (`M410`), then switch the laser off (`M5`).
pub const MARLIN_STOP: &[u8] = b"M410\nM5\n";

/// Real-time commands (single bytes, processed immediately, never buffered).
pub mod realtime {
    pub const STATUS_QUERY: u8 = b'?';
    pub const FEED_HOLD: u8 = b'!';
    pub const CYCLE_START: u8 = b'~';
    pub const SOFT_RESET: u8 = 0x18;
}

pub trait Transport {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Returns the next full line (without terminator), or `None` if nothing arrived within `timeout`.
    fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>>;
}

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("I/O error talking to the machine: {0}")]
    Io(#[from] io::Error),
    #[error("the controller rejected line {line} ({text:?}) with error:{code} {hint}")]
    Grbl { code: u32, line: usize, text: String, hint: String },
    #[error("the controller raised ALARM:{0} {1}; clear it (unlock/home) before running again")]
    Alarm(u32, String),
    #[error("line {line} is {len} bytes; GRBL accepts at most {} per line", stream::MAX_LINE_BYTES)]
    LineTooLong { line: usize, len: usize },
    #[error("line {line} contains byte 0x{byte:02x}, which the controller would act on as a real-time command or cannot parse")]
    ForbiddenByte { line: usize, byte: u8 },
    #[error("the machine entered an alarm state; clear it (unlock/home) before running again")]
    MachineAlarm,
    #[error("no response from the controller for {0} s")]
    Timeout(u64),
    #[error("the controller reported: {0}")]
    Reported(String),
    #[error("job cancelled")]
    Cancelled,
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn error_messages_explain_grbls_numbers_but_keep_them() {
        let e = DeviceError::Grbl { code: 22, line: 3, text: "G1 X5".into(), hint: error_hint(22) };
        assert_eq!(e.to_string(), "the controller rejected line 3 (\"G1 X5\") with error:22 (no feed rate has been set)");
        assert!(DeviceError::Alarm(3, alarm_hint(3)).to_string().contains("ALARM:3 (the machine was reset while moving"));
        assert!(
            DeviceError::Alarm(999, alarm_hint(999)).to_string().starts_with("the controller raised ALARM:999 (unknown code);"),
            "an unknown number stays a number"
        );
        assert!(
            DeviceError::Grbl { code: 999, line: 1, text: String::new(), hint: error_hint(999) }
                .to_string()
                .ends_with("with error:999 (unknown code)")
        );
    }
}
