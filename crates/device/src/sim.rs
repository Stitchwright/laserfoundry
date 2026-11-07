use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::time::Duration;

use crate::{GRBL_RX_BUFFER, Transport, realtime};

/// An in-memory GRBL. It models the 128-byte RX buffer (so over-sending is detectable) and answers
/// one buffered line per poll, which makes the streamer actually pipeline lines.
#[derive(Debug, Default)]
pub struct SimulatedGrbl {
    pending: VecDeque<String>,
    pending_bytes: usize,
    responses: VecDeque<String>,
    received: Vec<String>,
    partial: String,
    max_used: usize,
    overrun: bool,
    reset: bool,
    silent: bool,
    fail_prefix: Option<String>,
    alarm_prefix: Option<String>,
    /// Machine position (mm).
    position: (f64, f64),
    /// Work-coordinate offset set by `G92`.
    offset: (f64, f64),
    relative: bool,
    held: bool,
    homed: bool,
    latency: Duration,
    settings: BTreeMap<u32, String>,
    /// Never sleep (for hosts without threads, which poll instead of waiting).
    non_blocking: bool,
    /// Report an alarm state in status reports once any motion line has been accepted.
    alarm_when_busy: bool,
    busy_until_reset: bool,
}

impl SimulatedGrbl {
    pub fn new() -> Self {
        SimulatedGrbl { settings: default_settings(), ..Self::default() }
    }

    /// After the first accepted line, status reports say `Alarm` (a limit switch tripping mid-job).
    pub fn alarm_after_last_line(&mut self) {
        self.alarm_when_busy = true;
    }

    /// Reports `Run` after the first accepted line until a soft reset: a machine still moving
    /// although every line has been acknowledged.
    pub fn stay_busy_until_reset(&mut self) {
        self.busy_until_reset = true;
    }

    /// Makes reads return at once instead of waiting out their timeout, and ignores the latency.
    pub fn set_non_blocking(&mut self, non_blocking: bool) {
        self.non_blocking = non_blocking;
    }

    /// The setting values the "controller" currently holds.
    pub fn settings(&self) -> &BTreeMap<u32, String> {
        &self.settings
    }

    /// Sets a setting before connecting, to model a controller configured differently from the defaults.
    pub fn set_setting(&mut self, id: u32, value: &str) {
        self.settings.insert(id, value.to_string());
    }

    /// Lines the "controller" has accepted, in order.
    pub fn received(&self) -> &[String] {
        &self.received
    }

    /// Peak RX buffer occupancy in bytes.
    pub fn max_buffer_used(&self) -> usize {
        self.max_used
    }

    pub fn overran_buffer(&self) -> bool {
        self.overrun
    }

    pub fn was_reset(&self) -> bool {
        self.reset
    }

    /// Machine position in mm.
    pub fn position(&self) -> (f64, f64) {
        self.position
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    pub fn is_homed(&self) -> bool {
        self.homed
    }

    /// Lines starting with `prefix` answer `error:20`.
    pub fn fail_on(&mut self, prefix: &str) {
        self.fail_prefix = Some(prefix.into());
    }

    /// Lines starting with `prefix` answer `ALARM:1`.
    pub fn alarm_on(&mut self, prefix: &str) {
        self.alarm_prefix = Some(prefix.into());
    }

    /// Delay per response, so progress is visible in the UI.
    pub fn set_latency(&mut self, latency: Duration) {
        self.latency = latency;
    }

    pub fn go_silent(&mut self) {
        self.silent = true;
    }

    fn process_one(&mut self) {
        let Some(line) = self.pending.pop_front() else { return };
        self.pending_bytes -= line.len() + 1;
        if self.alarm_prefix.as_deref().is_some_and(|p| line.starts_with(p)) {
            self.responses.push_back("ALARM:1".into());
        } else if self.fail_prefix.as_deref().is_some_and(|p| line.starts_with(p)) {
            self.responses.push_back("error:20".into());
        } else {
            self.apply(&line);
            self.responses.push_back("ok".into());
        }
        self.received.push(line);
    }

    /// A tiny subset of GRBL's behaviour: enough for jogging, homing, work zero and motion.
    fn apply(&mut self, line: &str) {
        let upper = line.to_ascii_uppercase();
        if upper == "$H" {
            self.position = (0.0, 0.0);
            self.homed = true;
            return;
        }
        let (body, jog) = match upper.strip_prefix("$J=") {
            Some(rest) => (rest.to_string(), true),
            None => (upper.clone(), false),
        };
        if upper == "$$" {
            for (id, value) in &self.settings {
                self.responses.push_back(format!("${id}={value}"));
            }
            return;
        }
        if !jog && let Some((id, value)) = crate::settings::parse_setting(&upper) {
            self.settings.insert(id, value);
            return;
        }
        if body.starts_with('$') {
            return;
        }
        let mut relative = jog || self.relative;
        if body.contains("G91") {
            relative = true;
            self.relative = !jog;
        }
        if body.contains("G90") {
            relative = false;
            self.relative = false;
        }
        let (mut x, mut y) = (None, None);
        for word in body.split_whitespace() {
            let mut chars = word.chars();
            let (Some(axis), value) = (chars.next(), chars.as_str()) else { continue };
            if let Ok(v) = value.parse::<f64>() {
                match axis {
                    'X' => x = Some(v),
                    'Y' => y = Some(v),
                    _ => {}
                }
            }
        }
        if body.contains("G92") {
            // The head's current position becomes the given work coordinate.
            self.offset = (self.position.0 - x.unwrap_or(0.0), self.position.1 - y.unwrap_or(0.0));
            return;
        }
        if relative {
            self.position.0 += x.unwrap_or(0.0);
            self.position.1 += y.unwrap_or(0.0);
        } else {
            // G53 addresses machine coordinates, ignoring the work offset.
            let offset = if body.contains("G53") { (0.0, 0.0) } else { self.offset };
            if let Some(x) = x {
                self.position.0 = offset.0 + x;
            }
            if let Some(y) = y {
                self.position.1 = offset.1 + y;
            }
        }
    }

    fn status_line(&self) -> String {
        let state = if self.alarm_when_busy && !self.received.is_empty() {
            "Alarm"
        } else if self.busy_until_reset && !self.received.is_empty() && !self.reset {
            "Run"
        } else if self.held {
            "Hold:0"
        } else {
            "Idle"
        };
        format!("<{state}|MPos:{:.3},{:.3},0.000|FS:0,0>", self.position.0, self.position.1)
    }
}

impl Transport for SimulatedGrbl {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        for &b in bytes {
            if b == realtime::SOFT_RESET {
                self.reset = true;
                self.held = false;
                self.pending.clear();
                self.pending_bytes = 0;
                continue;
            }
            if b == realtime::STATUS_QUERY {
                let status = self.status_line();
                self.responses.push_back(status);
                continue;
            }
            if b == realtime::FEED_HOLD {
                self.held = true;
                continue;
            }
            if b == realtime::CYCLE_START {
                self.held = false;
                continue;
            }
            if b == b'\n' {
                let line = std::mem::take(&mut self.partial);
                self.pending_bytes += line.len() + 1;
                self.pending.push_back(line);
            } else {
                self.partial.push(char::from(b));
            }
            self.max_used = self.max_used.max(self.pending_bytes + self.partial.len());
            if self.pending_bytes + self.partial.len() > GRBL_RX_BUFFER {
                self.overrun = true;
            }
        }
        Ok(())
    }

    fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        if self.silent {
            if !self.non_blocking {
                std::thread::sleep(timeout);
            }
            return Ok(None);
        }
        if self.responses.is_empty() && !self.held {
            if !self.non_blocking && !self.latency.is_zero() && !self.pending.is_empty() {
                std::thread::sleep(self.latency);
            }
            self.process_one();
        }
        let response = self.responses.pop_front();
        if response.is_none() && !self.non_blocking {
            // A real port blocks until the timeout; do the same so callers do not spin.
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
        }
        Ok(response)
    }
}

/// GRBL 1.1's stock values, with a laser-friendly `$30`/`$32`.
fn default_settings() -> BTreeMap<u32, String> {
    [
        (0, "10"),
        (1, "25"),
        (2, "0"),
        (3, "0"),
        (4, "0"),
        (5, "0"),
        (6, "0"),
        (10, "1"),
        (11, "0.010"),
        (12, "0.002"),
        (13, "0"),
        (20, "0"),
        (21, "0"),
        (22, "0"),
        (23, "0"),
        (24, "25.000"),
        (25, "500.000"),
        (26, "250"),
        (27, "1.000"),
        (30, "1000"),
        (31, "0"),
        (32, "1"),
        (100, "250.000"),
        (101, "250.000"),
        (102, "250.000"),
        (110, "500.000"),
        (111, "500.000"),
        (112, "500.000"),
        (120, "10.000"),
        (121, "10.000"),
        (122, "10.000"),
        (130, "200.000"),
        (131, "200.000"),
        (132, "200.000"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect()
}
