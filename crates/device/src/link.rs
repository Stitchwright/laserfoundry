//! A persistent connection to a GRBL controller: a worker thread owns the [`Transport`] and does
//! status polling, immediate commands (jog, home, unlock, set zero), job streaming with
//! pause/resume/stop, and keeps a console log. The UI only sends commands and reads snapshots.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use web_time::Instant;

use crate::settings;
use crate::stream::{JobStream, MAX_LINE_BYTES, Window};
use crate::{DeviceError, MARLIN_STOP, MachineState, Progress, Protocol, Status, Transport, parse_marlin_position, parse_status, realtime};

/// Console lines kept.
pub const CONSOLE_LIMIT: usize = 500;
/// How often the worker asks the controller for a status report.
const STATUS_INTERVAL: Duration = Duration::from_millis(200);
/// How long one worker iteration waits for a response.
const POLL: Duration = Duration::from_millis(10);
/// A running job fails if the controller says nothing for this long (unless paused).
const JOB_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest single jog step accepted, in mm.
pub const MAX_JOG_MM: f64 = 1000.0;
pub const MAX_FEED: f64 = 30_000.0;
/// Largest absolute jog coordinate accepted, in mm.
pub const MAX_TRAVEL_MM: f64 = 100_000.0;

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Idle,
    Running,
    Paused,
    /// Finished, with the number of lines streamed or the reason it stopped.
    Finished(Result<usize, String>),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub connected: bool,
    pub status: Option<Status>,
    pub job: JobState,
    pub progress: Option<Progress>,
    pub console: Vec<String>,
    /// `$n=value` settings reported by the controller, by number.
    pub settings: BTreeMap<u32, String>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum LinkError {
    #[error("a job is running; stop it first")]
    Busy,
    #[error("not connected")]
    Disconnected,
    #[error("{0}")]
    Rejected(String),
}

/// Worker iterations run per [`Link::tick`] on an inline link.
const INLINE_STEPS: usize = 40;

pub struct Link {
    protocol: Protocol,
    tx: Sender<Cmd>,
    shared: Arc<Mutex<Shared>>,
    handle: Option<JoinHandle<()>>,
    /// Set for links that run on the caller's thread (see [`Link::connect_inline`]).
    inline: Option<Mutex<Inline>>,
}

struct Inline {
    worker: Worker,
    finished: bool,
}

impl Link {
    pub fn connect(transport: Box<dyn Transport + Send>) -> Link {
        Self::connect_with(transport, Protocol::Grbl)
    }

    /// Connects to a controller that speaks `protocol`.
    pub fn connect_with(transport: Box<dyn Transport + Send>, protocol: Protocol) -> Link {
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        if let Ok(mut s) = shared.lock() {
            s.connected = true;
        }
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::spawn(move || Worker::new(transport, protocol, rx, worker_shared).run());
        Link { protocol, tx, shared, handle: Some(handle), inline: None }
    }

    /// A link with no thread of its own, for hosts that have none (the browser): the owner calls
    /// [`tick`](Link::tick) regularly, and the transport must never block (return `None` from
    /// `read_line` at once when nothing has arrived).
    pub fn connect_inline(transport: Box<dyn Transport + Send>) -> Link {
        Self::connect_inline_with(transport, Protocol::Grbl)
    }

    /// [`connect_inline`](Link::connect_inline) for a controller that speaks `protocol`.
    pub fn connect_inline_with(transport: Box<dyn Transport + Send>, protocol: Protocol) -> Link {
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        if let Ok(mut s) = shared.lock() {
            s.connected = true;
        }
        let worker = Worker::new(transport, protocol, rx, Arc::clone(&shared));
        Link { protocol, tx, shared, handle: None, inline: Some(Mutex::new(Inline { worker, finished: false })) }
    }

    /// The protocol this link speaks.
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// Does the work an inline link's thread would have done since the last call. Does nothing for
    /// ordinary links.
    pub fn tick(&self) {
        let Some(inline) = &self.inline else { return };
        let Ok(mut guard) = inline.lock() else { return };
        let inline = &mut *guard;
        for _ in 0..INLINE_STEPS {
            if inline.finished {
                break;
            }
            if !inline.worker.iteration() {
                inline.worker.finish();
                inline.finished = true;
            }
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        match self.shared.lock() {
            Ok(s) => Snapshot {
                connected: s.connected,
                status: s.status.clone(),
                job: s.job.clone(),
                progress: s.progress,
                console: s.console.iter().cloned().collect(),
                settings: s.settings.clone(),
            },
            Err(_) => Snapshot {
                connected: false,
                status: None,
                job: JobState::Idle,
                progress: None,
                console: vec!["link state unavailable".into()],
                settings: BTreeMap::new(),
            },
        }
    }

    pub fn is_job_active(&self) -> bool {
        matches!(self.snapshot().job, JobState::Running | JobState::Paused)
    }

    /// Sends one G-code / `$` command line as-is (not while a job is running).
    pub fn send_line(&self, line: &str) -> Result<(), LinkError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        if line.len() > MAX_LINE_BYTES || line.contains(['\n', '\r', '\0']) || crate::stream::forbidden_byte(line).is_some() {
            return Err(LinkError::Rejected("not a valid single G-code line".into()));
        }
        self.require_idle()?;
        self.send(Cmd::Line(line.to_string()))
    }

    /// Sends several single-command lines in order (not while a job is running). Every line is checked before any is sent.
    pub fn send_lines(&self, lines: &[String]) -> Result<(), LinkError> {
        let lines: Vec<String> = lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if lines.iter().any(|l| l.len() > MAX_LINE_BYTES || l.contains(['\n', '\r', '\0']) || crate::stream::forbidden_byte(l).is_some()) {
            return Err(LinkError::Rejected("not a valid single G-code line".into()));
        }
        self.require_idle()?;
        self.send(Cmd::Lines(lines))
    }

    /// `$$`: ask the controller to report its settings (they arrive in [`Snapshot::settings`]). GRBL only.
    pub fn request_settings(&self) -> Result<(), LinkError> {
        self.require_grbl("settings")?;
        self.send_line("$$")
    }

    /// Changes one controller setting (`$id=value`) and asks for the settings again. GRBL only.
    pub fn write_setting(&self, id: u32, value: f64) -> Result<(), LinkError> {
        self.require_grbl("settings")?;
        self.send_line(&settings::setting_line(id, value)?)?;
        self.request_settings()
    }

    /// Relative jog in *machine* axes (`$J=G91 G21 ...`; for Marlin `G91`, a move, `G90`, with the laser off). Steps and feed are bounded.
    pub fn jog(&self, dx: f64, dy: f64, feed: f64) -> Result<(), LinkError> {
        let line = jog_line(dx, dy, feed)?;
        match self.protocol {
            Protocol::Grbl => self.send_line(&line),
            Protocol::Marlin => {
                let (x, y) = (
                    if dx != 0.0 { format!(" X{}", trim_num(dx)) } else { String::new() },
                    if dy != 0.0 { format!(" Y{}", trim_num(dy)) } else { String::new() },
                );
                self.send_lines(&["M5".into(), "G91".into(), format!("G1{x}{y} F{}", trim_num(feed)), "G90".into()])
            }
        }
    }

    /// Jog to an absolute position (GRBL: machine coordinates, `$J=G90 G21 G53 ...`; Marlin has no `G53` by default, so the
    /// position is in the work coordinates in force, with the laser off). Coordinates and feed are bounded.
    pub fn jog_to(&self, x: f64, y: f64, feed: f64) -> Result<(), LinkError> {
        let line = jog_to_line(x, y, feed)?;
        match self.protocol {
            Protocol::Grbl => self.send_line(&line),
            Protocol::Marlin => self.send_lines(&["M5".into(), "G90".into(), format!("G1 X{} Y{} F{}", trim_num(x), trim_num(y), trim_num(feed))]),
        }
    }

    /// Run the homing cycle (`$H`; Marlin `G28 X Y`).
    pub fn home(&self) -> Result<(), LinkError> {
        match self.protocol {
            Protocol::Grbl => self.send_line("$H"),
            Protocol::Marlin => self.send_line("G28 X Y"),
        }
    }

    /// Clear an alarm lock (`$X`; Marlin `M999`, which restarts it after a halt).
    pub fn unlock(&self) -> Result<(), LinkError> {
        match self.protocol {
            Protocol::Grbl => self.send_line("$X"),
            Protocol::Marlin => self.send_line("M999"),
        }
    }

    /// `G92 X0 Y0`: make the head's current position the work origin.
    pub fn set_zero(&self) -> Result<(), LinkError> {
        self.send_line("G92 X0 Y0")
    }

    /// Rapid move back to the work origin (Marlin runs one command per line, so `G90` and the move go separately).
    pub fn go_to_zero(&self) -> Result<(), LinkError> {
        match self.protocol {
            Protocol::Grbl => self.send_line("G90 G0 X0 Y0"),
            Protocol::Marlin => self.send_lines(&["M5".into(), "G90".into(), "G0 X0 Y0".into()]),
        }
    }

    pub fn start_job(&self, program: String) -> Result<(), LinkError> {
        self.require_idle()?;
        self.send(Cmd::Start(program))
    }

    pub fn pause(&self) -> Result<(), LinkError> {
        self.send(Cmd::Pause)
    }

    pub fn resume(&self) -> Result<(), LinkError> {
        self.send(Cmd::Resume)
    }

    /// Stops the machine: GRBL is soft-reset (motion stops and the laser goes off); Marlin is sent `M410` (stop, clear the
    /// planner) and `M5` (laser off).
    pub fn stop(&self) -> Result<(), LinkError> {
        self.send(Cmd::Stop)
    }

    fn require_grbl(&self, what: &str) -> Result<(), LinkError> {
        if self.protocol == Protocol::Grbl {
            Ok(())
        } else {
            Err(LinkError::Rejected(format!("this controller has no {what} to read or change here")))
        }
    }

    fn require_idle(&self) -> Result<(), LinkError> {
        if self.is_job_active() { Err(LinkError::Busy) } else { Ok(()) }
    }

    fn send(&self, cmd: Cmd) -> Result<(), LinkError> {
        self.tx.send(cmd).map_err(|_| LinkError::Disconnected)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // Never leave a laser job running unattended when its owner goes away.
        let _ = self.tx.send(Cmd::Close);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        // An inline link has no thread to notice the request: process it here.
        self.tick();
        if let Some(inline) = &self.inline
            && let Ok(mut guard) = inline.lock()
            && !guard.finished
        {
            guard.worker.finish();
            guard.finished = true;
        }
    }
}

/// Builds a GRBL jog command, refusing non-finite or huge values.
pub fn jog_line(dx: f64, dy: f64, feed: f64) -> Result<String, LinkError> {
    let ok = |v: f64| v.is_finite() && v.abs() <= MAX_JOG_MM;
    if !(ok(dx) && ok(dy)) || !(feed.is_finite() && (1.0..=MAX_FEED).contains(&feed)) {
        return Err(LinkError::Rejected(format!("jog steps must be within ±{MAX_JOG_MM} mm and feed within 1–{MAX_FEED} mm/min")));
    }
    let mut line = String::from("$J=G91 G21");
    if dx != 0.0 {
        line.push_str(&format!(" X{}", trim_num(dx)));
    }
    if dy != 0.0 {
        line.push_str(&format!(" Y{}", trim_num(dy)));
    }
    line.push_str(&format!(" F{}", trim_num(feed)));
    Ok(line)
}

/// Builds an absolute GRBL jog command, refusing non-finite or absurd values.
pub fn jog_to_line(x: f64, y: f64, feed: f64) -> Result<String, LinkError> {
    let ok = |v: f64| v.is_finite() && v.abs() <= MAX_TRAVEL_MM;
    if !(ok(x) && ok(y)) || !(feed.is_finite() && (1.0..=MAX_FEED).contains(&feed)) {
        return Err(LinkError::Rejected(format!("a jog target must be within ±{MAX_TRAVEL_MM} mm and feed within 1–{MAX_FEED} mm/min")));
    }
    // G53: machine coordinates, whatever work offsets (G54, G92) are in force.
    Ok(format!("$J=G90 G21 G53 X{} Y{} F{}", trim_num(x), trim_num(y), trim_num(feed)))
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.into() }
}

enum Cmd {
    Line(String),
    Lines(Vec<String>),
    Start(String),
    Pause,
    Resume,
    Stop,
    Close,
}

struct Shared {
    connected: bool,
    status: Option<Status>,
    job: JobState,
    progress: Option<Progress>,
    console: VecDeque<String>,
    settings: BTreeMap<u32, String>,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { connected: false, status: None, job: JobState::Idle, progress: None, console: VecDeque::new(), settings: BTreeMap::new() }
    }
}

/// What an `ok` that is not for a streamed job line answers, in the order the lines were sent.
enum Pending {
    /// A line the person sent.
    Line(String),
    /// A Marlin position request.
    Poll,
    /// A line the worker sent for its own reasons (the stop commands).
    Quiet,
    /// Marlin's `M400`, answered once the last queued move has finished: the job is over.
    Finish,
}

struct Worker {
    transport: Box<dyn Transport + Send>,
    protocol: Protocol,
    rx: Receiver<Cmd>,
    shared: Arc<Mutex<Shared>>,
    job: Option<JobStream>,
    /// Immediate lines sent and not yet acknowledged.
    pending: VecDeque<Pending>,
    paused: bool,
    finishing: bool,
    last_status_request: Instant,
    last_activity: Instant,
}

impl Worker {
    fn new(transport: Box<dyn Transport + Send>, protocol: Protocol, rx: Receiver<Cmd>, shared: Arc<Mutex<Shared>>) -> Self {
        let now = Instant::now();
        Worker {
            transport,
            protocol,
            rx,
            shared,
            job: None,
            pending: VecDeque::new(),
            paused: false,
            finishing: false,
            last_status_request: now - STATUS_INTERVAL,
            last_activity: now,
        }
    }

    fn run(mut self) {
        while self.iteration() {}
        self.finish();
    }

    /// One pass of the worker loop; `false` when it is time to stop.
    fn iteration(&mut self) -> bool {
        match self.drain_commands() {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                self.fail_io(e);
                return false;
            }
        }
        if let Err(e) = self.step() {
            if matches!(e, DeviceError::Io(_)) {
                // The connection itself is gone; there is nothing left to talk to.
                self.fail_io(e);
                return false;
            }
            self.fail_job(e);
        }
        true
    }

    /// Closing: stop motion and the laser if a job was in progress.
    fn finish(&mut self) {
        if self.job.is_some() {
            let _ = self.stop_machine();
        }
        self.update(|s| s.connected = false);
    }

    /// Stops motion and switches the laser off: GRBL is soft-reset, Marlin gets `M410` and `M5`.
    fn stop_machine(&mut self) -> Result<(), DeviceError> {
        match self.protocol {
            Protocol::Grbl => self.transport.write_all(&[realtime::SOFT_RESET])?,
            Protocol::Marlin => {
                self.transport.write_all(MARLIN_STOP)?;
                // Each is answered `ok`; nothing is waiting for those.
                self.pending.push_back(Pending::Quiet);
                self.pending.push_back(Pending::Quiet);
            }
        }
        Ok(())
    }

    /// Sends one line the person asked for and waits for its answer in the background.
    fn send_immediate(&mut self, line: String) -> Result<(), DeviceError> {
        self.log(format!("> {line}"));
        self.transport.write_all(format!("{line}\n").as_bytes())?;
        self.pending.push_back(Pending::Line(line));
        Ok(())
    }

    /// Returns `Ok(false)` when asked to close.
    fn drain_commands(&mut self) -> Result<bool, DeviceError> {
        loop {
            match self.rx.try_recv() {
                Ok(cmd) => {
                    if !self.handle(cmd)? {
                        return Ok(false);
                    }
                }
                Err(TryRecvError::Empty) => return Ok(true),
                Err(TryRecvError::Disconnected) => return Ok(false),
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) -> Result<bool, DeviceError> {
        match cmd {
            Cmd::Close => return Ok(false),
            Cmd::Line(line) => {
                if self.job.is_some() {
                    self.log("! ignored: a job is running");
                } else {
                    self.send_immediate(line)?;
                }
            }
            Cmd::Lines(lines) => {
                if self.job.is_some() {
                    self.log("! ignored: a job is running");
                } else {
                    for line in lines {
                        self.send_immediate(line)?;
                    }
                }
            }
            Cmd::Start(program) => {
                if self.job.is_some() {
                    self.log("! a job is already running");
                } else {
                    let window = match self.protocol {
                        Protocol::Grbl => Window::Bytes,
                        // Marlin acknowledges a line once it is queued and tells the host nothing about free buffer space,
                        // so one line at a time is the only flow control there is.
                        Protocol::Marlin => Window::Lines(1),
                    };
                    match JobStream::with_window(&program, window) {
                        Ok(job) => {
                            self.log(format!("job started: {} lines", job.progress().total));
                            self.job = Some(job);
                            self.paused = false;
                            self.finishing = false;
                            self.last_activity = Instant::now();
                            self.update(|s| s.job = JobState::Running);
                        }
                        Err(e) => self.finish_job(Err(e.to_string())),
                    }
                }
            }
            Cmd::Pause if self.job.is_some() && !self.paused => {
                match self.protocol {
                    Protocol::Grbl => {
                        self.transport.write_all(&[realtime::FEED_HOLD])?;
                        self.log("job paused (feed hold)");
                    }
                    // Marlin has no feed hold: no more lines are sent, and what it has already queued still runs.
                    Protocol::Marlin => self.log("job paused: no more lines are sent; moves already queued in the controller still finish"),
                }
                self.paused = true;
                self.update(|s| s.job = JobState::Paused);
            }
            Cmd::Resume if self.job.is_some() && self.paused => {
                if self.protocol == Protocol::Grbl {
                    self.transport.write_all(&[realtime::CYCLE_START])?;
                }
                self.paused = false;
                self.last_activity = Instant::now();
                self.log("job resumed");
                self.update(|s| s.job = JobState::Running);
            }
            Cmd::Pause | Cmd::Resume => {}
            Cmd::Stop => {
                self.pending.clear();
                self.stop_machine()?;
                self.finishing = false;
                if self.job.take().is_some() {
                    self.paused = false;
                    self.finish_job(Err(DeviceError::Cancelled.to_string()));
                }
                match self.protocol {
                    Protocol::Grbl => self.log("stopped (soft reset); if the machine reports an alarm, use Unlock"),
                    Protocol::Marlin => self.log("stopped (quick stop, laser off); if the controller halted, restart it"),
                }
            }
        }
        Ok(true)
    }

    fn step(&mut self) -> Result<(), DeviceError> {
        if self.last_status_request.elapsed() >= STATUS_INTERVAL {
            match self.protocol {
                Protocol::Grbl => {
                    self.transport.write_all(&[realtime::STATUS_QUERY])?;
                    self.last_status_request = Instant::now();
                }
                // Marlin answers a position request in line with everything else, so it is asked for only when nothing else is
                // outstanding (never during a job) and one request at a time.
                Protocol::Marlin if self.job.is_none() && self.pending.is_empty() => {
                    self.transport.write_all(b"M114\n")?;
                    self.pending.push_back(Pending::Poll);
                    self.last_status_request = Instant::now();
                }
                Protocol::Marlin => {}
            }
        }
        if let Some(job) = self.job.as_mut()
            && !self.finishing
            && !(self.paused && self.protocol == Protocol::Marlin)
        {
            job.pump(self.transport.as_mut())?;
            let p = job.progress();
            self.update(|s| s.progress = Some(p));
        }
        match self.transport.read_line(POLL)? {
            Some(line) => {
                self.last_activity = Instant::now();
                self.on_line(line.trim())
            }
            None => {
                let waiting = self.job.as_ref().is_some_and(|j| j.has_in_flight());
                if waiting && !self.paused && self.last_activity.elapsed() > JOB_IDLE_TIMEOUT {
                    return Err(DeviceError::Timeout(JOB_IDLE_TIMEOUT.as_secs()));
                }
                Ok(())
            }
        }
    }

    fn on_line(&mut self, line: &str) -> Result<(), DeviceError> {
        if line.is_empty() {
            return Ok(());
        }
        if self.protocol == Protocol::Marlin {
            return self.on_marlin_line(line);
        }
        if let Some(status) = parse_status(line) {
            let idle = status.state == crate::MachineState::Idle;
            let alarm = status.state == crate::MachineState::Alarm;
            self.update(|s| s.status = Some(status));
            if alarm && self.job.is_some() {
                // An alarm ends the job even when every line was already accepted.
                return Err(DeviceError::MachineAlarm);
            }
            if self.finishing && idle {
                let lines = self.job.as_ref().map_or(0, |j| j.progress().total);
                self.job = None;
                self.finishing = false;
                self.log("job finished");
                self.finish_job(Ok(lines));
            }
            return Ok(());
        }
        if line == "ok" || line.starts_with("error:") || line.starts_with("ALARM:") {
            if let Some(job) = self.job.as_mut()
                && job.has_in_flight()
            {
                if job.on_response(line)? {
                    let p = job.progress();
                    let done = job.is_done();
                    self.update(|s| s.progress = Some(p));
                    if done {
                        // Everything is accepted; wait for the machine to finish moving.
                        self.finishing = true;
                    }
                }
                return Ok(());
            }
            if line.starts_with("ALARM:") && self.job.is_some() {
                let code = line.trim_start_matches("ALARM:").trim().parse().unwrap_or(0);
                return Err(DeviceError::Alarm(code, crate::alarm_hint(code)));
            }
            let sent = match self.pending.pop_front() {
                Some(Pending::Line(s)) => Some(s),
                _ => None,
            };
            // GRBL's numbers, in words (a hint: other firmware may differ).
            let code = |prefix: &str| line.strip_prefix(prefix).and_then(|c| c.trim().parse::<u32>().ok());
            let hint = match (code("error:"), code("ALARM:")) {
                (Some(c), _) => crate::error_text(c).map(|t| format!(" ({t})")),
                (_, Some(c)) => crate::alarm_text(c).map(|t| format!(" ({t})")),
                _ => None,
            }
            .unwrap_or_default();
            self.log(match sent {
                Some(s) if line != "ok" => format!("< {line}{hint}   (for: {s})"),
                _ => format!("< {line}{hint}"),
            });
            if line.starts_with("ALARM:") {
                self.log("the machine is in an alarm state: use Unlock ($X), or Home ($H)");
            }
            return Ok(());
        }
        if let Some((id, value)) = settings::parse_setting(line) {
            self.update(|s| {
                if s.settings.len() < settings::MAX_SETTINGS || s.settings.contains_key(&id) {
                    s.settings.insert(id, value);
                }
            });
            return Ok(());
        }
        self.log(format!("< {line}"));
        Ok(())
    }

    /// One line from a Marlin controller: `ok` (for a streamed line, a position request, the end-of-job `M400` or something typed),
    /// `Error:`, `echo:` chatter or a position report.
    fn on_marlin_line(&mut self, line: &str) -> Result<(), DeviceError> {
        let is_ok = line == "ok" || line.strip_prefix("ok").is_some_and(|rest| rest.starts_with(' '));
        if is_ok {
            // Answers come in the order the lines were sent, so one owed for something sent before the job began (a position
            // request, a stop, a typed line) is used up first.
            if let Some(Pending::Line(_) | Pending::Poll | Pending::Quiet) = self.pending.front()
                && let Some(owed) = self.pending.pop_front()
            {
                if let Pending::Line(sent) = owed {
                    self.log(format!("< ok   (for: {sent})"));
                }
                return Ok(());
            }
            if let Some(job) = self.job.as_mut()
                && job.has_in_flight()
            {
                if job.on_response("ok")? {
                    let p = job.progress();
                    let done = job.is_done();
                    self.update(|s| s.progress = Some(p));
                    if done && !self.finishing {
                        // Everything is queued; `M400` is answered once the last move has finished.
                        self.finishing = true;
                        self.transport.write_all(b"M400\n")?;
                        self.pending.push_back(Pending::Finish);
                    }
                }
                return Ok(());
            }
            match self.pending.pop_front() {
                Some(Pending::Finish) if self.job.is_some() => {
                    let lines = self.job.as_ref().map_or(0, |j| j.progress().total);
                    self.job = None;
                    self.finishing = false;
                    self.log("job finished");
                    self.finish_job(Ok(lines));
                }
                // A stray `ok` (or the end-of-job answer with no job): nothing is waiting for it.
                Some(_) | None => {}
            }
            return Ok(());
        }
        let starts = |prefix: &str| line.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix));
        if starts("Error:") || starts("echo:Unknown command") {
            // Either the controller refused something or halted, or it does not know a command we sent (a G-code dialect it lacks):
            // a running job must not carry on past that with the laser doing something other than what was meant.
            if self.job.is_some() {
                return Err(DeviceError::Reported(line.to_string()));
            }
            self.log(format!("< {line}"));
            return Ok(());
        }
        if let Some(position) = parse_marlin_position(line) {
            let state = if self.job.is_some() { MachineState::Run } else { MachineState::Idle };
            // Marlin reports the position in the work coordinates in force, which shift with `G92`.
            self.update(|s| s.status = Some(Status { state, position: Some(position), position_is_work: true }));
            return Ok(());
        }
        // Marlin says `echo:busy: processing` every few seconds during a long move; that is not worth a console line.
        if !line.starts_with("echo:busy") {
            self.log(format!("< {line}"));
        }
        Ok(())
    }

    fn fail_job(&mut self, e: DeviceError) {
        self.log(format!("! {e}"));
        if self.job.take().is_some() {
            // An aborted job must not leave motion queued or the laser on.
            let _ = self.stop_machine();
            self.paused = false;
            self.finishing = false;
            self.finish_job(Err(e.to_string()));
        }
    }

    fn fail_io(&mut self, e: DeviceError) {
        self.log(format!("! connection error: {e}"));
        if self.job.is_some() {
            // A failed write does not always mean the port is gone: try to stop the machine before giving up on it.
            let _ = self.stop_machine();
        }
        self.job = None;
        self.update(|s| {
            s.connected = false;
            if matches!(s.job, JobState::Running | JobState::Paused) {
                s.job = JobState::Finished(Err(e.to_string()));
            }
        });
    }

    fn finish_job(&mut self, result: Result<usize, String>) {
        self.update(|s| s.job = JobState::Finished(result));
    }

    fn log(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.update(|s| {
            s.console.push_back(text);
            while s.console.len() > CONSOLE_LIMIT {
                s.console.pop_front();
            }
        });
    }

    fn update(&self, f: impl FnOnce(&mut Shared)) {
        if let Ok(mut s) = self.shared.lock() {
            f(&mut s);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::{MachineState, SimulatedGrbl};

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    fn sim_link() -> Link {
        let mut sim = SimulatedGrbl::new();
        sim.set_latency(Duration::from_millis(1));
        Link::connect(Box::new(sim))
    }

    fn wait_for(link: &Link, what: &str, mut ok: impl FnMut(&Snapshot) -> bool) -> Snapshot {
        let start = Instant::now();
        loop {
            let s = link.snapshot();
            if ok(&s) {
                return s;
            }
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}; console: {:?}", s.console);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn position(s: &Snapshot) -> Option<(f64, f64)> {
        s.status.as_ref().and_then(|st| st.position).map(|(x, y, _)| (x, y))
    }

    fn long_program(lines: usize) -> String {
        (0..lines).map(|i| format!("G1 X{} Y{} F1000\n", i % 100, i % 50)).collect()
    }

    mod immediate_tests {
        use super::*;

        #[test]
        fn status_is_polled_and_jog_moves_the_head() {
            let link = sim_link();
            wait_for(&link, "first status", |s| s.status.is_some());
            link.jog(10.0, -2.5, 3000.0).unwrap();
            let s = wait_for(&link, "jog to land", |s| position(s) == Some((10.0, -2.5)));
            assert_eq!(s.status.unwrap().state, MachineState::Idle);
            assert!(s.console.iter().any(|l| l == "> $J=G91 G21 X10 Y-2.5 F3000"), "{:?}", s.console);
        }

        #[test]
        fn absolute_jogs_land_on_the_target_and_bad_ones_are_refused() {
            assert_eq!(jog_to_line(10.0, 20.5, 3000.0).unwrap(), "$J=G90 G21 G53 X10 Y20.5 F3000");
            for (x, y, f) in [(f64::NAN, 0.0, 1000.0), (0.0, f64::INFINITY, 1000.0), (1e9, 0.0, 1000.0), (0.0, 0.0, 0.0), (0.0, 0.0, 1e9)] {
                assert!(jog_to_line(x, y, f).is_err(), "{x} {y} {f}");
            }
            let link = sim_link();
            // A work offset must not shift a machine-coordinate jog.
            link.set_zero().unwrap();
            link.jog_to(12.0, 7.0, 3000.0).unwrap();
            wait_for(&link, "the head to arrive", |s| position(s) == Some((12.0, 7.0)));
        }

        #[test]
        fn home_and_set_zero_and_go_to_zero() {
            let link = sim_link();
            link.jog(30.0, 40.0, 3000.0).unwrap();
            wait_for(&link, "jog", |s| position(s) == Some((30.0, 40.0)));
            link.set_zero().unwrap(); // work zero here
            link.jog(5.0, 5.0, 3000.0).unwrap();
            wait_for(&link, "second jog", |s| position(s) == Some((35.0, 45.0)));
            link.go_to_zero().unwrap();
            wait_for(&link, "return to the work origin", |s| position(s) == Some((30.0, 40.0)));
            link.home().unwrap();
            wait_for(&link, "homing", |s| position(s) == Some((0.0, 0.0)));
        }

        #[test]
        fn stopping_at_the_end_of_a_job_stays_cancelled_and_an_alarm_after_the_last_line_ends_the_job() {
            // A machine that has accepted every line but is still moving: Stop must not be undone
            // by the Idle report that follows the reset.
            let mut busy = SimulatedGrbl::new();
            busy.stay_busy_until_reset();
            let link = Link::connect(Box::new(busy));
            link.start_job(long_program(3)).unwrap();
            wait_for(&link, "every line to be accepted", |s| s.progress.is_some_and(|p| p.acknowledged == 3));
            assert_eq!(link.snapshot().job, JobState::Running, "still moving, so not finished");
            link.stop().unwrap();
            let s = wait_for(&link, "the stop", |s| matches!(s.job, JobState::Finished(Err(_))));
            std::thread::sleep(Duration::from_millis(500)); // several Idle reports arrive
            assert!(matches!(link.snapshot().job, JobState::Finished(Err(_))), "{:?}", link.snapshot().job);
            assert!(s.connected);

            // An alarm once every line has been accepted ends the job instead of leaving it Running.
            let mut sim = SimulatedGrbl::new();
            sim.alarm_after_last_line();
            let link = Link::connect(Box::new(sim));
            link.start_job(long_program(3)).unwrap();
            let s = wait_for(&link, "the alarm to end the job", |s| matches!(s.job, JobState::Finished(_)));
            assert!(matches!(s.job, JobState::Finished(Err(_))), "{:?}", s.job);
        }

        #[test]
        fn errors_from_the_controller_reach_the_console() {
            let mut sim = SimulatedGrbl::new();
            sim.fail_on("$J=");
            let link = Link::connect(Box::new(sim));
            link.jog(1.0, 0.0, 1000.0).unwrap();
            let s = wait_for(&link, "the error", |s| s.console.iter().any(|l| l.starts_with("< error:20")));
            assert!(s.console.iter().any(|l| l.contains("(for: $J=G91 G21 X1 F1000)")), "{:?}", s.console);
            assert!(
                s.console.iter().any(|l| l.starts_with("< error:20 (an unsupported or invalid G-code command)")),
                "the code is explained: {:?}",
                s.console
            );
        }

        #[test]
        fn console_lines_cannot_smuggle_realtime_bytes() {
            let link = sim_link();
            for bad in ["G0 X1 ?", "!", "~", "G0\u{18}", "$J=G91 X1\u{e9}"] {
                assert!(matches!(link.send_line(bad), Err(LinkError::Rejected(_))), "{bad:?}");
            }
        }

        #[test]
        fn alarms_explain_how_to_recover() {
            let mut sim = SimulatedGrbl::new();
            sim.alarm_on("G0");
            let link = Link::connect(Box::new(sim));
            link.send_line("G0 X1").unwrap();
            wait_for(&link, "the alarm hint", |s| s.console.iter().any(|l| l.contains("Unlock")));
        }
    }

    mod validation_tests {
        use super::*;

        #[test]
        fn jog_lines_are_bounded_and_formatted() {
            assert_eq!(jog_line(0.0, 5.0, 1200.0).unwrap(), "$J=G91 G21 Y5 F1200");
            assert_eq!(jog_line(-0.1, 0.0, 600.0).unwrap(), "$J=G91 G21 X-0.1 F600");
            for bad in [(f64::NAN, 0.0, 100.0), (2000.0, 0.0, 100.0), (1.0, 0.0, 0.0), (1.0, 0.0, f64::INFINITY), (1.0, 0.0, 1e9)] {
                assert!(jog_line(bad.0, bad.1, bad.2).is_err(), "{bad:?}");
            }
        }

        #[test]
        fn lines_that_could_smuggle_commands_are_rejected() {
            let link = sim_link();
            assert!(link.send_line("G0 X1\nM3 S1000").is_err());
            assert!(link.send_line("G0 X1\r").is_ok(), "a trailing CR is trimmed, not smuggled");
            assert!(link.send_line(&"X".repeat(200)).is_err());
            assert!(link.send_line("G0\u{18}X1").is_err(), "an embedded soft-reset byte is refused");
            assert!(link.send_line("   ").is_ok());
        }
    }

    mod job_tests {
        use super::*;

        #[test]
        fn a_job_runs_to_completion_and_waits_for_idle() {
            let link = sim_link();
            link.start_job(long_program(60)).unwrap();
            let s = wait_for(&link, "the job to finish", |s| matches!(s.job, JobState::Finished(_)));
            assert_eq!(s.job, JobState::Finished(Ok(60)));
            assert_eq!(s.progress.map(|p| p.acknowledged), Some(60));
        }

        #[test]
        fn immediate_commands_are_refused_while_a_job_runs() {
            let link = sim_link();
            link.start_job(long_program(2000)).unwrap();
            wait_for(&link, "the job to start", |s| s.job == JobState::Running);
            assert_eq!(link.send_line("G0 X1"), Err(LinkError::Busy));
            assert_eq!(link.jog(1.0, 0.0, 1000.0), Err(LinkError::Busy));
            assert_eq!(link.start_job("G21".into()), Err(LinkError::Busy));
            link.stop().unwrap();
            wait_for(&link, "stop", |s| matches!(s.job, JobState::Finished(_)));
        }

        #[test]
        fn pause_holds_progress_and_resume_finishes() {
            let link = sim_link();
            link.start_job(long_program(400)).unwrap();
            wait_for(&link, "some progress", |s| s.progress.is_some_and(|p| p.acknowledged > 5));
            link.pause().unwrap();
            let held =
                wait_for(&link, "the hold", |s| s.job == JobState::Paused && s.status.as_ref().is_some_and(|st| st.state == MachineState::Hold));
            let at = held.progress.unwrap().acknowledged;
            std::thread::sleep(Duration::from_millis(150));
            let later = link.snapshot().progress.unwrap().acknowledged;
            assert!(later <= at + 1, "progress must stop while held ({at} -> {later})");
            link.resume().unwrap();
            let s = wait_for(&link, "the job to finish", |s| matches!(s.job, JobState::Finished(_)));
            assert_eq!(s.job, JobState::Finished(Ok(400)));
        }

        #[test]
        fn stop_cancels_and_the_link_stays_usable() {
            let link = sim_link();
            link.start_job(long_program(5000)).unwrap();
            wait_for(&link, "some progress", |s| s.progress.is_some_and(|p| p.acknowledged > 3));
            link.stop().unwrap();
            let s = wait_for(&link, "stop", |s| matches!(s.job, JobState::Finished(_)));
            assert_eq!(s.job, JobState::Finished(Err("job cancelled".into())));
            assert!(s.connected);
            link.jog(1.0, 1.0, 1000.0).unwrap();
            wait_for(&link, "a jog after stop", |s| s.console.iter().any(|l| l.contains("$J=")));
        }

        #[test]
        fn a_controller_error_aborts_the_job_with_the_line_and_resets() {
            let mut sim = SimulatedGrbl::new();
            sim.fail_on("G1 X3 ");
            let link = Link::connect(Box::new(sim));
            link.start_job("G21\nG1 X1 F100\nG1 X3 Y3\nG1 X4\n".into()).unwrap();
            let s = wait_for(&link, "failure", |s| matches!(s.job, JobState::Finished(_)));
            match s.job {
                JobState::Finished(Err(m)) => assert!(m.contains("line 3") && m.contains("error:20"), "{m}"),
                other => panic!("expected failure, got {other:?}"),
            }
        }

        #[test]
        fn dropping_a_link_mid_job_returns_promptly() {
            let link = sim_link();
            link.start_job(long_program(5000)).unwrap();
            wait_for(&link, "the job to start", |s| s.progress.is_some());
            let start = Instant::now();
            drop(link);
            assert!(start.elapsed() < Duration::from_secs(2));
        }

        #[test]
        fn an_invalid_program_fails_cleanly() {
            let link = sim_link();
            link.start_job(format!("G1 {}", "X1 ".repeat(40))).unwrap();
            let s = wait_for(&link, "failure", |s| matches!(s.job, JobState::Finished(_)));
            assert!(matches!(s.job, JobState::Finished(Err(_))));
        }
    }

    mod inline_tests {
        use super::*;

        /// Ticks until `ok` holds; an inline link makes no progress by itself.
        fn tick_until(link: &Link, what: &str, mut ok: impl FnMut(&Snapshot) -> bool) -> Snapshot {
            let start = Instant::now();
            loop {
                link.tick();
                let s = link.snapshot();
                if ok(&s) {
                    return s;
                }
                assert!(start.elapsed() < WAIT, "timed out waiting for {what}; console: {:?}", s.console);
            }
        }

        #[test]
        fn an_inline_link_needs_a_tick_to_do_anything_and_then_behaves_like_a_threaded_one() {
            let link = Link::connect_inline(Box::new(SimulatedGrbl::new()));
            assert!(link.snapshot().status.is_none(), "nothing happens without ticks");
            tick_until(&link, "a status", |s| s.status.is_some());
            link.jog(10.0, 5.0, 3000.0).unwrap();
            tick_until(&link, "the jog", |s| position(s) == Some((10.0, 5.0)));
            link.request_settings().unwrap();
            let s = tick_until(&link, "settings", |s| s.settings.len() > 20);
            assert!(s.connected);
        }

        #[test]
        fn a_job_runs_to_the_end_and_can_be_stopped() {
            let link = Link::connect_inline(Box::new(SimulatedGrbl::new()));
            link.start_job(long_program(60)).unwrap();
            tick_until(&link, "the job to finish", |s| matches!(s.job, JobState::Finished(Ok(_))));

            link.start_job(long_program(5000)).unwrap();
            tick_until(&link, "the job to run", |s| s.job == JobState::Running);
            assert_eq!(link.send_line("G0 X1"), Err(LinkError::Busy));
            link.stop().unwrap();
            let s = tick_until(&link, "the stop", |s| matches!(s.job, JobState::Finished(Err(_))));
            assert!(s.connected, "stopping keeps the connection");
        }

        #[test]
        fn a_write_error_in_the_middle_of_a_job_still_tries_a_soft_reset() {
            use std::sync::{Arc, Mutex};
            /// Fails one write (the third line of the job) and takes everything else, remembering what it was given.
            struct Flaky(Arc<Mutex<(usize, Vec<u8>)>>);
            impl Transport for Flaky {
                fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
                    let mut state = self.0.lock().unwrap();
                    state.0 += 1;
                    if state.0 == 3 {
                        return Err(io::Error::other("glitch"));
                    }
                    state.1.extend_from_slice(bytes);
                    Ok(())
                }

                fn read_line(&mut self, _: Duration) -> io::Result<Option<String>> {
                    Ok(None)
                }
            }
            let seen = Arc::new(Mutex::new((0, Vec::new())));
            let link = Link::connect_inline(Box::new(Flaky(Arc::clone(&seen))));
            link.start_job(long_program(50)).unwrap();
            tick_until(&link, "the disconnect", |s| !s.connected);
            assert!(seen.lock().unwrap().1.contains(&realtime::SOFT_RESET), "a soft reset must be attempted after the write failed");
        }

        #[test]
        fn a_broken_transport_disconnects_and_dropping_a_running_inline_link_does_not_hang() {
            struct Broken;
            impl Transport for Broken {
                fn write_all(&mut self, _: &[u8]) -> io::Result<()> {
                    Err(io::Error::other("unplugged"))
                }

                fn read_line(&mut self, _: Duration) -> io::Result<Option<String>> {
                    Err(io::Error::other("unplugged"))
                }
            }
            let link = Link::connect_inline(Box::new(Broken));
            tick_until(&link, "the disconnect", |s| !s.connected);
            drop(link);
            let link = Link::connect_inline(Box::new(SimulatedGrbl::new()));
            link.start_job(long_program(5000)).unwrap();
            tick_until(&link, "the job to run", |s| s.job == JobState::Running);
            drop(link);
        }
    }

    mod settings_tests {
        use super::*;

        #[test]
        fn requesting_settings_fills_the_snapshot_and_keeps_them_out_of_the_console() {
            let link = sim_link();
            link.request_settings().unwrap();
            let s = wait_for(&link, "the settings report", |s| s.settings.len() > 20);
            assert_eq!(s.settings.get(&32).map(String::as_str), Some("1"));
            assert_eq!(s.settings.get(&130).map(String::as_str), Some("200.000"));
            assert!(!s.console.iter().any(|l| l.starts_with("< $")), "{:?}", s.console);
            wait_for(&link, "the ok", |s| s.console.iter().any(|l| l == "< ok"));
        }

        #[test]
        fn writing_a_setting_changes_it_and_reports_the_new_value() {
            let link = sim_link();
            link.write_setting(130, 410.5).unwrap();
            let s = wait_for(&link, "the new value", |s| s.settings.get(&130).is_some_and(|v| v == "410.5"));
            assert!(s.console.iter().any(|l| l == "> $130=410.5"), "{:?}", s.console);
        }

        #[test]
        fn settings_are_refused_while_a_job_runs() {
            let mut sim = SimulatedGrbl::new();
            sim.set_latency(Duration::from_millis(20));
            let link = Link::connect(Box::new(sim));
            link.start_job(long_program(200)).unwrap();
            wait_for(&link, "the job to run", |s| s.job == JobState::Running);
            assert_eq!(link.write_setting(30, 255.0), Err(LinkError::Busy));
            assert_eq!(link.request_settings(), Err(LinkError::Busy));
            link.stop().unwrap();
        }

        #[test]
        fn a_controller_cannot_flood_the_settings_table() {
            struct Flood(u32);
            impl Transport for Flood {
                fn write_all(&mut self, _: &[u8]) -> io::Result<()> {
                    Ok(())
                }

                fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
                    if self.0 < 5000 {
                        self.0 += 1;
                        return Ok(Some(format!("${}=1", self.0 % 1000)));
                    }
                    std::thread::sleep(timeout);
                    Ok(None)
                }
            }
            let link = Link::connect(Box::new(Flood(0)));
            let s = wait_for(&link, "the flood to be read", |s| s.settings.len() >= crate::settings::MAX_SETTINGS);
            assert!(s.settings.len() <= crate::settings::MAX_SETTINGS);
        }
    }

    mod connection_tests {
        use super::*;

        struct Broken;

        impl Transport for Broken {
            fn write_all(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::Error::other("unplugged"))
            }

            fn read_line(&mut self, _: Duration) -> io::Result<Option<String>> {
                Err(io::Error::other("unplugged"))
            }
        }

        #[test]
        fn a_vanished_device_marks_the_link_disconnected_instead_of_spinning() {
            let link = Link::connect(Box::new(Broken));
            let s = wait_for(&link, "disconnect", |s| !s.connected);
            assert!(s.console.iter().any(|l| l.contains("connection error")), "{:?}", s.console);
            assert_eq!(link.send_line("G0 X1"), Err(LinkError::Disconnected), "commands after a disconnect report it");
        }
    }

    mod marlin_tests {
        use std::sync::{Arc, Mutex};

        use crate::SimulatedMarlin;

        use super::*;

        /// What a scripted controller has been sent, and the answers it will give (one per read).
        #[derive(Default)]
        struct Wire {
            written: Vec<u8>,
            replies: VecDeque<String>,
        }

        struct Scripted(Arc<Mutex<Wire>>);

        impl Transport for Scripted {
            fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
                self.0.lock().unwrap().written.extend_from_slice(bytes);
                Ok(())
            }

            fn read_line(&mut self, _: Duration) -> io::Result<Option<String>> {
                Ok(self.0.lock().unwrap().replies.pop_front())
            }
        }

        /// Lets a test look at a simulated Marlin that the link owns.
        struct Spy(Arc<Mutex<SimulatedMarlin>>);

        impl Transport for Spy {
            fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
                self.0.lock().unwrap().write_all(bytes)
            }

            fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
                self.0.lock().unwrap().read_line(timeout)
            }
        }

        fn scripted() -> (Link, Arc<Mutex<Wire>>) {
            let wire = Arc::new(Mutex::new(Wire::default()));
            (Link::connect_inline_with(Box::new(Scripted(Arc::clone(&wire))), Protocol::Marlin), wire)
        }

        fn simulated() -> (Link, Arc<Mutex<SimulatedMarlin>>) {
            let mut sim = SimulatedMarlin::new();
            sim.set_latency(Duration::from_millis(1));
            let sim = Arc::new(Mutex::new(sim));
            (Link::connect_with(Box::new(Spy(Arc::clone(&sim))), Protocol::Marlin), sim)
        }

        /// The complete lines written so far.
        fn sent(wire: &Arc<Mutex<Wire>>) -> Vec<String> {
            let w = wire.lock().unwrap();
            let text = String::from_utf8_lossy(&w.written).into_owned();
            text.strip_suffix('\n').unwrap_or(&text).split('\n').filter(|l| !l.is_empty()).map(String::from).collect()
        }

        fn reply(wire: &Arc<Mutex<Wire>>, line: &str) {
            wire.lock().unwrap().replies.push_back(line.into());
        }

        /// Runs the worker a while with nothing arriving.
        fn settle(link: &Link) {
            for _ in 0..30 {
                link.tick();
            }
        }

        /// Answers the position request every idle Marlin link starts with, so later answers line up.
        fn answer_first_poll(link: &Link, wire: &Arc<Mutex<Wire>>) {
            settle(link);
            assert_eq!(sent(wire), vec!["M114"], "an idle link asks for its position first");
            reply(wire, "X:0.00 Y:0.00 Z:0.00 E:0.00 Count X:0 Y:0 Z:0");
            reply(wire, "ok");
            settle(link);
        }

        /// How many command words (G or M plus a number) a line has: Marlin runs only the first.
        fn commands_in(line: &str) -> usize {
            line.split_whitespace()
                .filter(|w| w.len() > 1 && matches!(w.as_bytes()[0], b'G' | b'M') && w[1..].bytes().all(|b| b.is_ascii_digit()))
                .count()
        }

        fn program() -> String {
            "G21\nM3 I S0\nG0 X10 Y10\nG1 X20 Y10 S100 F1000\nM5\nG0 X0 Y0\n".into()
        }

        #[test]
        fn an_idle_link_reports_the_position_and_marks_it_as_work_coordinates() {
            let (link, wire) = scripted();
            assert_eq!(link.protocol(), Protocol::Marlin);
            answer_first_poll(&link, &wire);
            let s = link.snapshot();
            let status = s.status.expect("a status");
            assert_eq!((status.state, status.position, status.position_is_work), (MachineState::Idle, Some((0.0, 0.0, 0.0)), true));
            assert!(s.console.is_empty(), "position requests and their answers stay out of the console: {:?}", s.console);
        }

        #[test]
        fn only_one_position_request_is_outstanding_however_silent_the_controller_is() {
            let (link, wire) = scripted();
            for _ in 0..20 {
                settle(&link);
                std::thread::sleep(Duration::from_millis(15));
            }
            assert_eq!(sent(&wire), vec!["M114"], "no flood of requests to a controller that has not answered");
        }

        #[test]
        fn a_job_is_streamed_one_line_at_a_time_and_ends_only_when_the_last_move_has_finished() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.start_job(program()).unwrap();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M114", "G21"], "one line out, waiting for its ok");
            assert_eq!(link.snapshot().job, JobState::Running);
            for (n, expected) in ["M3 I S0", "G0 X10 Y10", "G1 X20 Y10 S100 F1000", "M5", "G0 X0 Y0"].iter().enumerate() {
                reply(&wire, "ok");
                settle(&link);
                assert_eq!(sent(&wire).last().map(String::as_str), Some(*expected), "after {} oks", n + 1);
                assert_eq!(sent(&wire).len(), n + 3, "never two lines in flight");
            }
            reply(&wire, "ok");
            settle(&link);
            assert_eq!(sent(&wire).last().map(String::as_str), Some("M400"), "wait for the moves to finish");
            assert_eq!(link.snapshot().job, JobState::Running, "all lines queued is not yet finished");
            assert_eq!(link.snapshot().progress.map(|p| (p.acknowledged, p.total)), Some((6, 6)));
            reply(&wire, "ok");
            settle(&link);
            assert_eq!(link.snapshot().job, JobState::Finished(Ok(6)));
            assert!(sent(&wire).iter().all(|l| commands_in(l) <= 1), "{:?}", sent(&wire));
        }

        #[test]
        fn acknowledgements_with_extra_fields_count_and_lookalikes_do_not() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.start_job("G21\nG90\n".into()).unwrap();
            settle(&link);
            reply(&wire, "okay");
            reply(&wire, "ok1");
            reply(&wire, "OK");
            settle(&link);
            assert_eq!(sent(&wire), vec!["M114", "G21"], "none of those is an ok");
            reply(&wire, "ok P15 B3");
            settle(&link);
            assert_eq!(sent(&wire).last().map(String::as_str), Some("G90"));
        }

        #[test]
        fn pausing_sends_nothing_more_and_resuming_carries_on() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.start_job(program()).unwrap();
            settle(&link);
            link.pause().unwrap();
            settle(&link);
            assert_eq!(link.snapshot().job, JobState::Paused);
            reply(&wire, "ok");
            settle(&link);
            assert_eq!(sent(&wire), vec!["M114", "G21"], "the answer to the line in flight does not release another while paused");
            assert!(!wire.lock().unwrap().written.iter().any(|b| matches!(b, b'!' | b'~' | 0x18)), "no GRBL real-time bytes");
            link.resume().unwrap();
            settle(&link);
            assert_eq!(link.snapshot().job, JobState::Running);
            assert_eq!(sent(&wire).last().map(String::as_str), Some("M3 I S0"));
        }

        #[test]
        fn stopping_sends_a_quick_stop_and_laser_off_and_never_a_reset_byte() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.start_job(program()).unwrap();
            settle(&link);
            link.stop().unwrap();
            settle(&link);
            assert_eq!(&sent(&wire)[2..], ["M410", "M5"], "{:?}", sent(&wire));
            assert!(matches!(link.snapshot().job, JobState::Finished(Err(_))));
            assert!(!wire.lock().unwrap().written.iter().any(|b| matches!(b, b'!' | b'~' | b'?' | 0x18)), "no GRBL real-time bytes");
            // The link is usable again.
            link.set_zero().unwrap();
            settle(&link);
            assert_eq!(sent(&wire).last().map(String::as_str), Some("G92 X0 Y0"));
        }

        #[test]
        fn an_error_or_an_unknown_command_during_a_job_ends_it_with_the_laser_stopped() {
            for bad in ["Error:Printer halted. kill() called!", "error:something", "echo:Unknown command: \"M3\""] {
                let (link, wire) = scripted();
                answer_first_poll(&link, &wire);
                link.start_job(program()).unwrap();
                settle(&link);
                reply(&wire, bad);
                settle(&link);
                let s = link.snapshot();
                assert!(matches!(&s.job, JobState::Finished(Err(why)) if why.contains(bad)), "{bad}: {:?}", s.job);
                assert_eq!(&sent(&wire)[2..], ["M410", "M5"], "{bad}: {:?}", sent(&wire));
            }
        }

        #[test]
        fn errors_and_chatter_when_idle_are_logged_and_change_nothing() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            for line in [
                "Error:Line Number is not Last Line Number+1",
                "echo:Unknown command: \"M9\"",
                "echo:busy: processing",
                "Marlin 2.1.2",
                "\u{1}\u{2}junk",
            ] {
                reply(&wire, line);
            }
            settle(&link);
            let s = link.snapshot();
            assert_eq!(s.job, JobState::Idle);
            assert!(s.connected);
            assert!(s.console.iter().any(|l| l.contains("Line Number")), "{:?}", s.console);
            assert!(!s.console.iter().any(|l| l.contains("busy")), "busy chatter is not worth a console line: {:?}", s.console);
        }

        #[test]
        fn dropping_a_link_mid_job_stops_the_machine() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.start_job(program()).unwrap();
            settle(&link);
            drop(link);
            assert_eq!(&sent(&wire)[2..], ["M410", "M5"], "{:?}", sent(&wire));
        }

        #[test]
        fn an_answer_still_owed_from_before_the_job_is_not_taken_for_the_first_lines() {
            let (link, wire) = scripted();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M114"]);
            // The job starts before the position request has been answered.
            link.start_job("G21\nG90\n".into()).unwrap();
            settle(&link);
            assert_eq!(sent(&wire), vec!["M114", "G21"]);
            reply(&wire, "ok"); // the position request's
            settle(&link);
            assert_eq!(sent(&wire), vec!["M114", "G21"], "that ok is not G21's");
            assert_eq!(link.snapshot().progress.map(|p| p.acknowledged), Some(0));
            reply(&wire, "ok"); // G21's
            settle(&link);
            assert_eq!(sent(&wire).last().map(String::as_str), Some("G90"));
        }

        #[test]
        fn immediate_commands_are_single_commands_and_the_laser_goes_off_first() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.jog(10.0, -2.5, 3000.0).unwrap();
            link.jog_to(40.0, 25.0, 6000.0).unwrap();
            link.home().unwrap();
            link.unlock().unwrap();
            link.set_zero().unwrap();
            link.go_to_zero().unwrap();
            settle(&link);
            let lines: Vec<String> = sent(&wire)[1..].to_vec();
            assert_eq!(
                lines,
                ["M5", "G91", "G1 X10 Y-2.5 F3000", "G90", "M5", "G90", "G1 X40 Y25 F6000", "G28 X Y", "M999", "G92 X0 Y0", "M5", "G90", "G0 X0 Y0"],
                "{lines:?}"
            );
            assert!(lines.iter().all(|l| commands_in(l) <= 1), "Marlin would run only the first command of {lines:?}");
            assert!(lines.iter().all(|l| !l.starts_with('$')), "no GRBL commands: {lines:?}");
        }

        #[test]
        fn grbl_settings_are_refused_and_bad_jogs_still_are() {
            let (link, _wire) = scripted();
            assert!(matches!(link.request_settings(), Err(LinkError::Rejected(_))));
            assert!(matches!(link.write_setting(30, 1000.0), Err(LinkError::Rejected(_))));
            for bad in [(f64::NAN, 0.0, 1000.0), (5000.0, 0.0, 1000.0), (1.0, 0.0, 0.0), (1.0, 0.0, f64::INFINITY)] {
                assert!(link.jog(bad.0, bad.1, bad.2).is_err(), "{bad:?}");
            }
            assert!(link.jog_to(f64::NAN, 0.0, 1000.0).is_err());
            assert!(link.send_line("G1 X1 ?").is_err() && link.send_line("G1\nM5").is_err());
            assert!(link.send_lines(&["G90".into(), "G1 X1 !".into()]).is_err(), "all or nothing");
        }

        #[test]
        fn no_grbl_real_time_byte_is_ever_sent_in_a_marlin_session() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            link.jog(1.0, 1.0, 1000.0).unwrap();
            link.start_job(program()).unwrap();
            for _ in 0..8 {
                reply(&wire, "ok");
                settle(&link);
            }
            link.pause().unwrap();
            link.resume().unwrap();
            link.stop().unwrap();
            settle(&link);
            let written = wire.lock().unwrap().written.clone();
            assert!(!written.iter().any(|b| matches!(b, b'?' | b'!' | b'~' | 0x18)), "{:?}", String::from_utf8_lossy(&written));
        }

        #[test]
        fn a_flood_of_junk_from_the_controller_cannot_break_the_link_or_grow_the_console_without_bound() {
            let (link, wire) = scripted();
            answer_first_poll(&link, &wire);
            for i in 0..5000 {
                reply(&wire, &format!("junk {i} {}", "x".repeat(i % 400)));
                reply(&wire, if i % 3 == 0 { "ok" } else { "<Idle|MPos:1,2,3>" });
            }
            for _ in 0..400 {
                link.tick();
            }
            let s = link.snapshot();
            assert!(s.connected && s.console.len() <= CONSOLE_LIMIT, "{} console lines", s.console.len());
        }

        #[test]
        fn a_whole_job_runs_against_the_simulated_controller_and_a_stop_leaves_the_laser_off() {
            let (link, sim) = simulated();
            wait_for(&link, "a position", |s| s.status.is_some());
            link.set_zero().unwrap();
            link.start_job(program()).unwrap();
            let s = wait_for(&link, "the job to finish", |s| matches!(s.job, JobState::Finished(_)));
            assert_eq!(s.job, JobState::Finished(Ok(6)), "{:?}", s.console);
            {
                let sim = sim.lock().unwrap();
                assert_eq!(sim.position(), (0.0, 0.0), "the job ends back at the origin");
                assert!(!sim.laser_enabled());
                let ran: Vec<&str> = sim.received().iter().map(String::as_str).filter(|l| *l != "M114").collect();
                assert_eq!(ran, ["G92 X0 Y0", "G21", "M3 I S0", "G0 X10 Y10", "G1 X20 Y10 S100 F1000", "M5", "G0 X0 Y0", "M400"]);
                assert!(sim.received().iter().all(|l| commands_in(l) <= 1 && !l.contains(['?', '!', '~', '\u{18}'])), "{:?}", sim.received());
            }
            // A second job, stopped while the laser is enabled.
            link.start_job("M3 I S0\nG1 X5 Y5 S200 F100\nG1 X50 Y50 S200 F100\nG1 X90 Y90 S200 F100\n".repeat(30)).unwrap();
            wait_for(&link, "the job to be under way", |s| s.progress.is_some_and(|p| p.acknowledged >= 2));
            link.stop().unwrap();
            wait_for(&link, "the stop", |s| matches!(s.job, JobState::Finished(Err(_))));
            std::thread::sleep(Duration::from_millis(50));
            let sim = sim.lock().unwrap();
            assert!(sim.was_quick_stopped() && !sim.laser_enabled(), "stopped: {}, laser on: {}", sim.was_quick_stopped(), sim.laser_enabled());
        }

        #[test]
        fn a_halted_printer_ends_the_job_and_a_silent_one_times_the_link_out_of_waiting_without_hanging() {
            let (link, sim) = simulated();
            sim.lock().unwrap().halt_on("G1");
            wait_for(&link, "a position", |s| s.status.is_some());
            link.start_job(program()).unwrap();
            let s = wait_for(&link, "the failure", |s| matches!(s.job, JobState::Finished(_)));
            assert!(matches!(&s.job, JobState::Finished(Err(why)) if why.contains("Printer halted")), "{:?}", s.job);
            // Dropping the link does not hang on a controller that has stopped answering.
            sim.lock().unwrap().go_silent();
            drop(link);
        }
    }
}
