use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use crate::Transport;

/// An in-memory Marlin, for tests and the in-app simulator. It models what matters to a host: each line is one command (only the
/// first command word on a line runs, as in Marlin), every line is answered `ok` once handled, `M114` reports the position, there
/// are no real-time commands (a stray `?` or reset byte simply becomes part of the line), `M410` clears the queue and `M5` switches
/// the laser off. Written from Marlin's public G-code index; it has never been compared with a real controller.
#[derive(Debug, Default)]
pub struct SimulatedMarlin {
    queue: VecDeque<String>,
    responses: VecDeque<String>,
    received: Vec<String>,
    partial: String,
    /// Machine position (mm) and the work offset set by `G92`.
    position: (f64, f64),
    offset: (f64, f64),
    relative: bool,
    homed: bool,
    laser_enabled: bool,
    quick_stopped: bool,
    halted: bool,
    advanced_ok: bool,
    silent: bool,
    fail_prefix: Option<String>,
    latency: Duration,
    non_blocking: bool,
}

impl SimulatedMarlin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lines starting with `prefix` halt the "printer": it answers `Error:Printer halted...` and then says nothing more.
    pub fn halt_on(&mut self, prefix: &str) {
        self.fail_prefix = Some(prefix.into());
    }

    /// Answers `ok P15 B3` instead of `ok`, as Marlin does with advanced acknowledgements.
    pub fn use_advanced_ok(&mut self) {
        self.advanced_ok = true;
    }

    pub fn go_silent(&mut self) {
        self.silent = true;
    }

    pub fn set_latency(&mut self, latency: Duration) {
        self.latency = latency;
    }

    /// Makes reads return at once instead of waiting out their timeout, and ignores the latency.
    pub fn set_non_blocking(&mut self, non_blocking: bool) {
        self.non_blocking = non_blocking;
    }

    /// Lines the "controller" has handled, in order (with anything stray that was sent mixed into them).
    pub fn received(&self) -> &[String] {
        &self.received
    }

    /// Position in work coordinates, as `M114` reports it.
    pub fn position(&self) -> (f64, f64) {
        (self.position.0 - self.offset.0, self.position.1 - self.offset.1)
    }

    /// True while a spindle/laser command (`M3`/`M4`) is in force and no `M5` has followed.
    pub fn laser_enabled(&self) -> bool {
        self.laser_enabled
    }

    pub fn was_quick_stopped(&self) -> bool {
        self.quick_stopped
    }

    pub fn is_homed(&self) -> bool {
        self.homed
    }

    fn process_one(&mut self) {
        let Some(line) = self.queue.pop_front() else { return };
        self.received.push(line.clone());
        if self.fail_prefix.as_deref().is_some_and(|p| line.starts_with(p)) {
            self.halted = true;
            self.responses.push_back("Error:Printer halted. kill() called!".into());
            return;
        }
        let ok = if self.advanced_ok { "ok P15 B3" } else { "ok" };
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("").to_ascii_uppercase();
        let params: Vec<String> = words.map(str::to_ascii_uppercase).collect();
        let number = |axis: char| params.iter().find_map(|w| w.strip_prefix(axis)?.parse::<f64>().ok());
        match command.as_str() {
            "" | "G21" | "M999" | "M400" | "M117" => {}
            "G28" => {
                self.position = (0.0, 0.0);
                self.homed = true;
            }
            "G90" => self.relative = false,
            "G91" => self.relative = true,
            "G92" => {
                self.offset = (self.position.0 - number('X').unwrap_or(0.0), self.position.1 - number('Y').unwrap_or(0.0));
            }
            "G0" | "G1" => {
                if self.relative {
                    self.position.0 += number('X').unwrap_or(0.0);
                    self.position.1 += number('Y').unwrap_or(0.0);
                } else {
                    if let Some(x) = number('X') {
                        self.position.0 = self.offset.0 + x;
                    }
                    if let Some(y) = number('Y') {
                        self.position.1 = self.offset.1 + y;
                    }
                }
            }
            "M3" | "M4" => self.laser_enabled = true,
            "M5" => self.laser_enabled = false,
            "M410" => {
                self.quick_stopped = true;
                self.queue.clear();
            }
            "M114" => {
                let (x, y) = self.position();
                self.responses.push_back(format!("X:{x:.2} Y:{y:.2} Z:0.00 E:0.00 Count X:{} Y:{} Z:0", (x * 80.0).round(), (y * 80.0).round()));
            }
            other => self.responses.push_back(format!("echo:Unknown command: \"{other}\"")),
        }
        self.responses.push_back(ok.into());
    }
}

impl Transport for SimulatedMarlin {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        for &b in bytes {
            // No real-time commands: every byte but the line end is part of the line.
            if b == b'\n' {
                let line = std::mem::take(&mut self.partial);
                self.queue.push_back(line);
            } else {
                self.partial.push(char::from(b));
            }
        }
        Ok(())
    }

    fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        if self.silent || self.halted {
            if !self.non_blocking {
                std::thread::sleep(timeout.min(Duration::from_millis(5)));
            }
            // A halted printer still reports what it had already said.
            return Ok(if self.halted { self.responses.pop_front() } else { None });
        }
        if self.responses.is_empty() {
            if !self.non_blocking && !self.latency.is_zero() && !self.queue.is_empty() {
                std::thread::sleep(self.latency);
            }
            self.process_one();
        }
        let response = self.responses.pop_front();
        if response.is_none() && !self.non_blocking {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
        }
        Ok(response)
    }
}
