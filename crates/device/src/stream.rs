use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use web_time::Instant;

use crate::{DeviceError, GRBL_RX_BUFFER, Transport, realtime};

/// GRBL's maximum line length including the terminator is 80; we keep a margin.
pub const MAX_LINE_BYTES: usize = 79;

#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// Give up if the controller says nothing for this long (long moves can take a while).
    pub idle_timeout: Duration,
    /// How long one poll waits for a response.
    pub poll: Duration,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self { idle_timeout: Duration::from_secs(60), poll: Duration::from_millis(20) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub sent: usize,
    pub acknowledged: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamReport {
    pub lines: usize,
}

/// How many lines the streamer may have outstanding at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    /// As many as fit GRBL's RX buffer (character counting).
    Bytes,
    /// At most this many, however short (a firmware with no way to count free buffer space).
    Lines(usize),
}

/// The protocol state of one program being streamed: which lines are in flight and how many
/// bytes of GRBL's RX buffer they occupy. Shared by [`stream_program`] and the persistent link.
pub(crate) struct JobStream {
    lines: Vec<String>,
    /// The program line number (1-based, as in the source text) of each prepared line.
    numbers: Vec<usize>,
    in_flight: VecDeque<usize>,
    in_flight_bytes: usize,
    next: usize,
    acked: usize,
    window: Window,
}

impl JobStream {
    pub(crate) fn new(program: &str) -> Result<Self, DeviceError> {
        Self::with_window(program, Window::Bytes)
    }

    pub(crate) fn with_window(program: &str, window: Window) -> Result<Self, DeviceError> {
        let (numbers, lines): (Vec<usize>, Vec<String>) = prepare_numbered(program)?.into_iter().unzip();
        Ok(Self { lines, numbers, in_flight: VecDeque::new(), in_flight_bytes: 0, next: 0, acked: 0, window })
    }

    pub(crate) fn progress(&self) -> Progress {
        Progress { sent: self.next, acknowledged: self.acked, total: self.lines.len() }
    }

    pub(crate) fn is_done(&self) -> bool {
        self.acked >= self.lines.len()
    }

    pub(crate) fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Sends as many lines as fit in the controller's RX buffer.
    pub(crate) fn pump(&mut self, transport: &mut dyn Transport) -> Result<(), DeviceError> {
        while let Some(line) = self.lines.get(self.next) {
            let cost = line.len() + 1;
            let full = match self.window {
                Window::Bytes => self.in_flight_bytes + cost > GRBL_RX_BUFFER - 1,
                Window::Lines(n) => self.in_flight.len() >= n.max(1),
            };
            if full {
                break;
            }
            transport.write_all(format!("{line}\n").as_bytes())?;
            self.in_flight.push_back(cost);
            self.in_flight_bytes += cost;
            self.next += 1;
        }
        Ok(())
    }

    /// Feeds one controller response. `Ok(true)` if it was an acknowledgement of a streamed line.
    pub(crate) fn on_response(&mut self, resp: &str) -> Result<bool, DeviceError> {
        let resp = resp.trim();
        if resp == "ok" {
            // An acknowledgement with nothing in flight (a leftover from before a reset, or a
            // duplicate) does not belong to any streamed line.
            let Some(cost) = self.in_flight.pop_front() else { return Ok(false) };
            self.in_flight_bytes = self.in_flight_bytes.saturating_sub(cost);
            self.acked += 1;
            Ok(true)
        } else if let Some(code) = resp.strip_prefix("error:") {
            let index = self.acked;
            let code: u32 = code.trim().parse().unwrap_or(0);
            Err(DeviceError::Grbl {
                code,
                hint: crate::error_hint(code),
                line: self.numbers.get(index).copied().unwrap_or(index + 1),
                text: self.lines.get(index).cloned().unwrap_or_default(),
            })
        } else if let Some(code) = resp.strip_prefix("ALARM:") {
            let code: u32 = code.trim().parse().unwrap_or(0);
            Err(DeviceError::Alarm(code, crate::alarm_hint(code)))
        } else {
            // Status reports, "[MSG:...]" and banners are informational.
            Ok(false)
        }
    }
}

/// Streams a G-code program with GRBL's character-counting protocol: keep as many lines in flight
/// as fit in the 128-byte RX buffer and free space as `ok`/`error` responses arrive.
pub fn stream_program(
    transport: &mut dyn Transport,
    program: &str,
    options: &StreamOptions,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<StreamReport, DeviceError> {
    let mut job = JobStream::new(program)?;
    let result = run_stream(transport, &mut job, options, cancel, &mut on_progress);
    if let Err(e) = &result
        && !matches!(e, DeviceError::Cancelled | DeviceError::Io(_))
    {
        // Giving up on a job must not leave queued moves running with the laser on.
        let _ = transport.write_all(&[realtime::SOFT_RESET]);
    }
    result
}

fn run_stream(
    transport: &mut dyn Transport,
    job: &mut JobStream,
    options: &StreamOptions,
    cancel: &AtomicBool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<StreamReport, DeviceError> {
    let mut last_activity = Instant::now();

    while !job.is_done() {
        if cancel.load(Ordering::Relaxed) {
            // Reset flushes GRBL's buffers and stops motion; the laser is switched off by the reset.
            transport.write_all(&[realtime::SOFT_RESET])?;
            return Err(DeviceError::Cancelled);
        }
        let before = job.progress().sent;
        job.pump(transport)?;
        if job.progress().sent != before {
            on_progress(job.progress());
        }
        match transport.read_line(options.poll)? {
            Some(resp) => {
                last_activity = Instant::now();
                if job.on_response(&resp)? {
                    on_progress(job.progress());
                }
            }
            None if last_activity.elapsed() > options.idle_timeout => return Err(DeviceError::Timeout(options.idle_timeout.as_secs())),
            None => {}
        }
    }
    Ok(StreamReport { lines: job.progress().total })
}

/// A byte that must never be sent as part of a G-code line: GRBL treats `?`, `!`, `~` and the soft
/// reset (0x18) as real-time commands wherever they appear (so a stray one could pause, resume or
/// reset the machine mid-job), and control, DEL and non-ASCII bytes are not valid G-code.
pub(crate) fn forbidden_byte(line: &str) -> Option<u8> {
    line.bytes().find(|b| matches!(b, b'?' | b'!' | b'~') || (*b < 0x20 && *b != b'\t') || *b >= 0x7f)
}

/// Strips comments and blank lines; refuses lines GRBL would truncate.
#[cfg(test)]
pub(crate) fn prepare(program: &str) -> Result<Vec<String>, DeviceError> {
    Ok(prepare_numbered(program)?.into_iter().map(|(_, l)| l).collect())
}

/// Lines of margin when restarting an interrupted job: the controller acknowledges a line when it
/// is buffered, not when it has run, so the last few acknowledged lines may never have happened.
pub const RESTART_BACKUP_LINES: usize = 20;

/// The 1-based program line to restart from after a job stopped with `acknowledged` lines
/// acknowledged: [`RESTART_BACKUP_LINES`] before the last acknowledged one, never before line 1.
/// Re-running a few lines is safe (the restart repositions with the beam off); skipping some is not.
pub fn restart_line(program: &str, acknowledged: usize) -> usize {
    let target = acknowledged.saturating_sub(RESTART_BACKUP_LINES);
    if target == 0 {
        return 1;
    }
    let mut seen = 0;
    for (i, raw) in program.lines().enumerate() {
        let code = strip_paren_comments(raw.split(';').next().unwrap_or(""));
        if code.trim().is_empty() {
            continue;
        }
        seen += 1;
        if seen == target {
            return i + 1;
        }
    }
    1
}

/// Like [`prepare`], keeping each line's number in the original program.
pub(crate) fn prepare_numbered(program: &str) -> Result<Vec<(usize, String)>, DeviceError> {
    let mut out = Vec::new();
    for (i, raw) in program.lines().enumerate() {
        // GRBL acts on these bytes wherever they appear, comments included.
        if let Some(byte) = forbidden_byte(raw) {
            return Err(DeviceError::ForbiddenByte { line: i + 1, byte });
        }
        let code = raw.split(';').next().unwrap_or("");
        let code = strip_paren_comments(code);
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        if code.len() > MAX_LINE_BYTES {
            return Err(DeviceError::LineTooLong { line: i + 1, len: code.len() });
        }
        out.push((i + 1, code.to_string()));
    }
    Ok(out)
}

fn strip_paren_comments(s: &str) -> String {
    let mut depth = 0u32;
    s.chars()
        .filter(|c| match c {
            '(' => {
                depth += 1;
                false
            }
            ')' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use crate::SimulatedGrbl;

    use super::*;

    fn program(n: usize) -> String {
        (0..n).map(|i| format!("G1 X{}.{:03} Y{} S500 F1000\n", i % 300, i % 1000, i % 200)).collect()
    }

    mod restart_tests {
        use super::*;

        #[test]
        fn restart_backs_up_over_buffered_lines_and_counts_code_lines_only() {
            let p = "; header\nG21\n\n(note)\nG1 X1\nG1 X2\n";
            // Nothing or little acknowledged: start at the top.
            assert_eq!(restart_line(p, 0), 1);
            assert_eq!(restart_line(p, 3), 1);
            // 25 acknowledged lines back up by RESTART_BACKUP_LINES to code line 5.
            let long: String = (0..100).map(|i| format!("; c{i}\nG1 X{i}\n")).collect();
            let line = restart_line(&long, 25);
            assert_eq!(line, 2 * (25 - RESTART_BACKUP_LINES));
            assert_eq!(long.lines().nth(line - 1), Some("G1 X4"));
            // More acknowledged than the program holds is harmless.
            assert_eq!(restart_line(&long, 10_000), 1);
            assert_eq!(restart_line("", 50), 1);
        }
    }

    mod flow_tests {
        use super::*;

        #[test]
        fn streams_a_long_program_without_overrunning_the_rx_buffer() {
            let mut sim = SimulatedGrbl::new();
            let report = stream_program(&mut sim, &program(500), &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap();
            assert_eq!(report.lines, 500);
            assert_eq!(sim.received().len(), 500);
            assert!(sim.max_buffer_used() <= GRBL_RX_BUFFER, "buffer peaked at {}", sim.max_buffer_used());
            assert!(sim.max_buffer_used() > 80, "should actually pipeline, peaked at {}", sim.max_buffer_used());
        }

        #[test]
        fn comments_and_blank_lines_are_not_sent() {
            let mut sim = SimulatedGrbl::new();
            stream_program(&mut sim, "; hi\n\nG21 (units)\nG90 ; abs\n", &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap();
            assert_eq!(sim.received(), ["G21", "G90"]);
        }

        #[test]
        fn progress_reaches_total() {
            let mut sim = SimulatedGrbl::new();
            let mut last = None;
            stream_program(&mut sim, &program(20), &StreamOptions::default(), &AtomicBool::new(false), |p| last = Some(p)).unwrap();
            assert_eq!(last, Some(Progress { sent: 20, acknowledged: 20, total: 20 }));
        }
    }

    mod realtime_byte_tests {
        use super::*;

        #[test]
        fn programs_with_realtime_or_odd_bytes_are_refused_before_anything_is_sent() {
            for bad in
                ["G0 X1\n\u{18}\nG0 X2\n", "G0 X1 (pause !)\n", "G1 X1 ; why?\n", "G0 X1~\n", "G0 X1\u{7}\n", "G0 X1 \u{e9}\n", "G0 X\u{7f}1\n"]
            {
                assert!(matches!(prepare(bad), Err(DeviceError::ForbiddenByte { .. })), "{bad:?}");
                let mut sim = SimulatedGrbl::new();
                let r = stream_program(&mut sim, bad, &StreamOptions::default(), &AtomicBool::new(false), |_| {});
                assert!(matches!(r, Err(DeviceError::ForbiddenByte { .. })), "{bad:?}");
                assert!(sim.received().is_empty() && !sim.was_reset(), "nothing reached the controller for {bad:?}");
            }
            assert!(prepare("G0 X1\tY2 (note)\n; comment\n").is_ok(), "tabs and ordinary comments are fine");
        }
    }

    mod failure_tests {
        use super::*;

        #[test]
        fn controller_errors_name_the_offending_line() {
            let mut sim = SimulatedGrbl::new();
            sim.fail_on("G1 X999");
            let err = stream_program(&mut sim, "G21\nG1 X999 Y0\nG0 X0\n", &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::Grbl { line: 2, .. }), "{err}");
        }

        #[test]
        fn error_lines_use_the_programs_own_numbering_even_after_comments_and_blanks() {
            let mut sim = SimulatedGrbl::new();
            sim.fail_on("G1 X999");
            let program = "; header\n; more header\n\nG21\n(note)\nG1 X999 Y0\nG0 X0\n";
            let err = stream_program(&mut sim, program, &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::Grbl { line: 6, .. }), "{err}");
        }

        #[test]
        fn giving_up_on_a_job_resets_the_controller_so_queued_moves_stop() {
            let mut sim = SimulatedGrbl::new();
            sim.fail_on("G1 X999");
            let _ = stream_program(&mut sim, "G21\nG1 X5 S900\nG1 X999\nG0 X0\n", &StreamOptions::default(), &AtomicBool::new(false), |_| {})
                .unwrap_err();
            assert!(sim.was_reset(), "an error must stop motion and switch the laser off");

            let mut sim = SimulatedGrbl::new();
            sim.alarm_on("G1");
            let _ = stream_program(&mut sim, "G1 X1\n", &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(sim.was_reset());

            let mut sim = SimulatedGrbl::new();
            sim.go_silent();
            let options = StreamOptions { idle_timeout: Duration::from_millis(60), ..StreamOptions::default() };
            let err = stream_program(&mut sim, "G1 X1\n", &options, &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::Timeout(_)));
            assert!(sim.was_reset(), "a controller that stops answering mid-job is reset too");
        }

        #[test]
        fn stray_acknowledgements_are_not_counted_as_streamed_lines() {
            let mut job = JobStream::new("G0 X1\nG0 X2\n").unwrap();
            assert!(!job.on_response("ok").unwrap(), "nothing is in flight");
            assert!(!job.is_done() && job.progress().acknowledged == 0);
            let mut sim = SimulatedGrbl::new();
            job.pump(&mut sim).unwrap();
            assert!(job.on_response("ok").unwrap());
            assert_eq!(job.progress().acknowledged, 1);
        }

        #[test]
        fn alarm_aborts() {
            let mut sim = SimulatedGrbl::new();
            sim.alarm_on("G1");
            let err = stream_program(&mut sim, "G1 X1\n", &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::Alarm(1, _)));
        }

        #[test]
        fn overlong_lines_are_refused_before_sending_anything() {
            let mut sim = SimulatedGrbl::new();
            let long = format!("G1 {}", "X1 ".repeat(40));
            let err = stream_program(&mut sim, &long, &StreamOptions::default(), &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::LineTooLong { .. }));
            assert!(sim.received().is_empty());
        }

        #[test]
        fn cancel_sends_soft_reset() {
            let mut sim = SimulatedGrbl::new();
            let cancel = AtomicBool::new(true);
            let err = stream_program(&mut sim, &program(10), &StreamOptions::default(), &cancel, |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::Cancelled));
            assert!(sim.was_reset());
        }

        #[test]
        fn silent_controller_times_out() {
            let mut sim = SimulatedGrbl::new();
            sim.go_silent();
            let opts = StreamOptions { idle_timeout: Duration::from_millis(50), poll: Duration::from_millis(5) };
            let err = stream_program(&mut sim, "G21\n", &opts, &AtomicBool::new(false), |_| {}).unwrap_err();
            assert!(matches!(err, DeviceError::Timeout(_)));
        }
    }
}
