//! The live link: lets an agent work in the document of a running desktop app instead of a private
//! session.
//!
//! The app opts in (it is off by default) and listens on the loopback interface only, on a port the
//! system picks. It writes the port and a random token to a per-user *link file* readable by the
//! owner alone; a client must present that token as its first line, so another local user, or a web
//! page poking at `localhost`, cannot drive the app. Connection threads only move lines: every
//! command runs on the app's own thread through [`LinkServer::serve_pending`], once per frame, so the
//! UI and the agent never touch the session at the same time. The client side, [`proxy`], bridges
//! an MCP client's stdio to the socket (`laserfoundry-cli mcp --connect`).
//!
//! Only engine commands cross the link. Starting a job on a machine is a UI action and is not
//! reachable, and commands that would replace unsaved work are refused ([`Mode::Live`]).

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use laserfoundry_engine::Session;
use serde_json::{Value, json};

use crate::{Line, MAX_MESSAGE_BYTES, Mode, Subscriptions, handle_line_with, read_capped_line, write_message};

/// Agents connected at once.
const MAX_CLIENTS: usize = 4;
/// Longest accepted handshake line, in bytes.
const MAX_HANDSHAKE_BYTES: usize = 512;
/// How long a new connection may take to present its token.
const HANDSHAKE_TIMEOUT: Duration = if cfg!(test) { Duration::from_millis(600) } else { Duration::from_secs(5) };
/// Requests answered per frame, so a flood cannot freeze the window.
const MAX_PER_PASS: usize = 16;
/// Bytes of randomness in the token.
const TOKEN_BYTES: usize = 32;
/// Requests waiting for the app, in all: a client that sends faster than the window answers waits its turn
/// instead of piling them up in memory.
const EVENT_QUEUE: usize = 256;
/// Replies waiting to be written to one agent. An agent that stops reading is disconnected when this fills.
const OUT_QUEUE: usize = if cfg!(test) { 4 } else { 256 };
/// How long writing to an agent may stall before it is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// What a connection thread tells the app thread, which owns the session and every connection's state.
enum Event {
    /// An agent passed the handshake; everything for it goes out through `out`.
    Open {
        id: u64,
        out: SyncSender<Value>,
    },
    Line {
        id: u64,
        line: String,
    },
    Close {
        id: u64,
    },
}

/// The app thread's view of one agent.
struct Connection {
    out: SyncSender<Value>,
    subs: Subscriptions,
}

/// A running listener, owned by the app. Dropping it closes the port and removes the link file.
pub struct LinkServer {
    events: Receiver<Event>,
    connections: HashMap<u64, Connection>,
    stop: Arc<AtomicBool>,
    addr: SocketAddr,
    link_file: PathBuf,
}

impl LinkServer {
    /// Opens a loopback port, writes the link file at `link_file` and starts accepting agents.
    /// `wake` is called (from another thread) whenever a request arrives, so the app can repaint.
    pub fn start(link_file: &Path, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let addr = listener.local_addr()?;
        let token = new_token()?;
        write_link_file(link_file, addr.port(), &token)?;

        let (tx, events) = mpsc::sync_channel(EVENT_QUEUE);
        let next_id = Arc::new(AtomicU64::new(1));
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(AtomicUsize::new(0));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let accept_stop = stop.clone();
        thread::Builder::new().name("laserfoundry-link".into()).spawn(move || {
            for stream in listener.incoming() {
                if accept_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                if clients.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                    clients.fetch_sub(1, Ordering::AcqRel);
                    continue; // dropped: too many agents
                }
                let (tx, token, wake, count, id) =
                    (tx.clone(), token.clone(), wake.clone(), clients.clone(), next_id.fetch_add(1, Ordering::Relaxed));
                let spawned = thread::Builder::new().name("laserfoundry-link-client".into()).spawn(move || {
                    if let Err(e) = client(stream, id, &token, &tx, wake.as_ref()) {
                        log::debug!("agent link closed: {e}");
                    }
                    count.fetch_sub(1, Ordering::AcqRel);
                });
                if spawned.is_err() {
                    clients.fetch_sub(1, Ordering::AcqRel); // the thread never ran, so it never gave its slot back
                    log::warn!("could not start a link thread");
                }
            }
        })?;
        Ok(Self { events, connections: HashMap::new(), stop, addr, link_file: link_file.to_path_buf() })
    }

    /// Where agents can connect.
    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    /// Handles what arrived since the last call (at most a frame's worth of requests), running it on
    /// `session`, then tells each agent about watched resources that changed, including by the
    /// person's own edits. Call every frame from the thread that owns the session. Returns how many
    /// requests were handled.
    pub fn serve_pending(&mut self, session: &mut Session) -> usize {
        let mut handled = 0;
        let mut dropped = Vec::new();
        while handled < MAX_PER_PASS {
            let Ok(event) = self.events.try_recv() else { break };
            match event {
                Event::Open { id, out } => {
                    self.connections.insert(id, Connection { out, subs: Subscriptions::default() });
                }
                Event::Close { id } => {
                    self.connections.remove(&id); // dropping `out` lets the writer finish what is queued
                }
                Event::Line { id, line } => {
                    handled += 1;
                    let Some(conn) = self.connections.get_mut(&id) else { continue };
                    if let Some(reply) = handle_line_with(session, line.trim(), Mode::Live, &mut conn.subs)
                        && conn.out.try_send(reply).is_err()
                    {
                        dropped.push(id); // gone, or not reading its replies
                    }
                }
            }
        }
        for (id, conn) in &mut self.connections {
            for note in conn.subs.changed(session) {
                if conn.out.try_send(note).is_err() {
                    dropped.push(*id);
                    break;
                }
            }
        }
        for id in dropped {
            self.connections.remove(&id); // its writer ends and hangs up
        }
        handled
    }
}

impl Drop for LinkServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200)); // wakes the accept loop
        // Another instance may have taken the file over since: only remove our own.
        let ours = fs::read_to_string(&self.link_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .is_some_and(|v| v.get("port").and_then(Value::as_u64) == Some(u64::from(self.addr.port())));
        if ours {
            let _ = fs::remove_file(&self.link_file);
        }
    }
}

/// One agent: checks the token, then forwards each line to the app. Answers and notifications come
/// back through a writer thread, in the order the app produced them.
fn client(stream: TcpStream, id: u64, token: &str, tx: &SyncSender<Event>, wake: &(dyn Fn() + Send + Sync)) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut input = BufReader::new(stream.try_clone()?);
    let mut output = stream.try_clone()?;

    // The read timeout applies to each read, so a client sending a byte at a time could hold the slot for ever:
    // give the whole handshake one deadline.
    let handshaken = Arc::new(AtomicBool::new(false));
    {
        let (handshaken, stream) = (Arc::clone(&handshaken), stream.try_clone()?);
        thread::Builder::new().name("laserfoundry-link-deadline".into()).spawn(move || {
            thread::sleep(HANDSHAKE_TIMEOUT);
            if !handshaken.load(Ordering::SeqCst) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        })?;
    }
    let mut buf = Vec::new();
    let line = read_capped_line(&mut input, &mut buf, MAX_HANDSHAKE_BYTES);
    handshaken.store(true, Ordering::SeqCst);
    match line? {
        Line::Ok => {}
        Line::End | Line::TooLong => return Err(io::Error::other("bad handshake")),
    }
    let presented = serde_json::from_slice::<Value>(&buf).ok().and_then(|v| v.get("token").and_then(Value::as_str).map(str::to_owned));
    if !presented.is_some_and(|p| same(p.as_bytes(), token.as_bytes())) {
        write_message(&mut output, &json!({ "ok": false, "error": "wrong token" }))?;
        return Err(io::Error::other("wrong token"));
    }
    write_message(&mut output, &json!({ "ok": true }))?;
    stream.set_read_timeout(None)?;

    let (out, queue) = mpsc::sync_channel::<Value>(OUT_QUEUE);
    thread::Builder::new().name("laserfoundry-link-writer".into()).spawn(move || {
        for message in queue {
            if write_message(&mut output, &message).is_err() {
                break;
            }
        }
        let _ = output.shutdown(Shutdown::Both);
    })?;
    let notice = out.clone();
    send(tx, wake, Event::Open { id, out })?;

    let result = (|| loop {
        buf.clear();
        match read_capped_line(&mut input, &mut buf, MAX_MESSAGE_BYTES)? {
            Line::End => return Ok(()),
            Line::TooLong => {
                let reply = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "message too large" } });
                let _ = notice.try_send(reply);
            }
            Line::Ok => {
                let line = String::from_utf8_lossy(&buf).into_owned();
                if !line.trim().is_empty() {
                    send(tx, wake, Event::Line { id, line })?;
                }
            }
        }
    })();
    let _ = send(tx, wake, Event::Close { id });
    result
}

fn send(tx: &SyncSender<Event>, wake: &(dyn Fn() + Send + Sync), event: Event) -> io::Result<()> {
    tx.send(event).map_err(|_| io::Error::other("app closed"))?;
    wake();
    Ok(())
}

/// Where the running app and `laserfoundry-cli mcp --connect` agree to meet: the per-user data folder.
pub fn default_link_file() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_RUNTIME_DIR").or_else(|| var("XDG_DATA_HOME")).or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    }?;
    Some(base.join("LaserFoundry").join("agent-link.json"))
}

/// Connects to the app named by `link_file` and relays MCP lines between `input`/`output` (an
/// agent's stdio) and it, until `input` ends.
pub fn proxy(link_file: &Path, input: impl BufRead, output: impl Write + Send + 'static) -> io::Result<()> {
    let text = fs::read_to_string(link_file)
        .map_err(|e| io::Error::new(e.kind(), format!("no running LaserFoundry to connect to ({}): {e}", link_file.display())))?;
    let info: Value = serde_json::from_str(&text).map_err(|e| io::Error::other(format!("unreadable link file: {e}")))?;
    let port =
        info.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).ok_or_else(|| io::Error::other("link file has no port"))?;
    let token = info.get("token").and_then(Value::as_str).ok_or_else(|| io::Error::other("link file has no token"))?;

    let stream = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(3))
        .map_err(|e| io::Error::new(e.kind(), format!("LaserFoundry is not listening (is agent access switched on?): {e}")))?;
    stream.set_nodelay(true)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    write_message(&mut writer, &json!({ "token": token }))?;
    let mut hello = String::new();
    reader.by_ref().take(MAX_HANDSHAKE_BYTES as u64).read_line(&mut hello)?;
    if serde_json::from_str::<Value>(&hello).ok().and_then(|v| v.get("ok").and_then(Value::as_bool)) != Some(true) {
        return Err(io::Error::other("LaserFoundry refused the connection (stale link file?)"));
    }

    let output = Arc::new(Mutex::new(output));
    let relay_out = output.clone();
    let relay = thread::spawn(move || {
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match read_capped_line(&mut reader, &mut buf, MAX_MESSAGE_BYTES) {
                Ok(Line::Ok) => {
                    let Ok(mut out) = relay_out.lock() else { return };
                    if out.write_all(&buf).and_then(|()| out.flush()).is_err() {
                        return;
                    }
                }
                Ok(Line::TooLong) => {}
                Ok(Line::End) | Err(_) => return,
            }
        }
    });

    let mut input = input;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match read_capped_line(&mut input, &mut buf, MAX_MESSAGE_BYTES)? {
            Line::End => break,
            Line::TooLong => continue,
            Line::Ok => {
                writer.write_all(&buf)?;
                writer.flush()?;
            }
        }
    }
    let _ = writer.shutdown(Shutdown::Write); // the app finishes pending answers, then closes
    let _ = relay.join();
    Ok(())
}

fn write_link_file(path: &Path, port: u16, token: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let _ = fs::remove_file(path); // so the owner-only mode applies to a fresh file
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(json!({ "port": port, "token": token, "pid": std::process::id() }).to_string().as_bytes())?;
    file.flush()
}

/// A random hex token from the operating system's generator. If that is unavailable there is no token,
/// and so no link: a guessable token would let another local process drive the app.
fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(format!("no secure random numbers for the link token: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    mod link_tests {
        use super::*;

        #[test]
        fn an_agent_edits_the_open_document_through_the_link() {
            let (file, _guard) = scratch("roundtrip");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            let mut session = Session::new();
            let (mut agent, mut lines) = connect(&file);
            writeln!(agent, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shape_rectangle","arguments":{"x":5,"y":5,"width":20,"height":10}}})).unwrap();
            let reply = pump(&mut server, &mut session, &mut lines);
            assert_eq!(reply["id"], json!(1));
            assert_eq!(reply["result"]["isError"], json!(false));
            assert_eq!(session.doc.shapes.len(), 1, "the agent's shape is in the window's document");
        }

        #[test]
        fn a_wrong_token_is_turned_away_and_changes_nothing() {
            let (file, _guard) = scratch("token");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            let mut s = TcpStream::connect(server.address()).unwrap();
            writeln!(s, r#"{{"token":"nope"}}"#).unwrap();
            let mut answer = String::new();
            BufReader::new(s.try_clone().unwrap()).read_line(&mut answer).unwrap();
            assert!(answer.contains("false"));
            let _ = writeln!(s, r#"{{"jsonrpc":"2.0","id":1,"method":"ping"}}"#);
            assert_eq!(server.serve_pending(&mut Session::new()), 0);
        }

        #[test]
        fn a_client_that_dribbles_its_handshake_is_cut_off_at_the_deadline() {
            let (file, _guard) = scratch("dribble");
            let server = LinkServer::start(&file, || {}).unwrap();
            let mut s = TcpStream::connect(server.address()).unwrap();
            let started = std::time::Instant::now();
            // One byte every 200 ms, never a line end: each read succeeds, so only a total deadline can stop it.
            let mut cut = false;
            for _ in 0..40 {
                if s.write_all(b"x").is_err() {
                    cut = true;
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
            assert!(cut, "the server kept a half-finished handshake for {:?}", started.elapsed());
            assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        }

        #[test]
        fn an_agent_that_never_reads_its_replies_is_dropped_instead_of_filling_memory() {
            let (file, _guard) = scratch("deaf");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            let mut session = Session::new();
            let (mut agent, _unread) = connect(&file);
            let request = json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": {"uri": "laserfoundry://reference/commands"}});
            // Ask for the (large) command reference over and over without ever reading an answer. The socket
            // buffers fill, the writer stalls, the queue fills, and the server hangs up on this agent.
            let started = std::time::Instant::now();
            let mut dropped = false;
            'asking: for _ in 0..200 {
                for _ in 0..50 {
                    if writeln!(agent, "{request}").is_err() {
                        dropped = true; // the server closed the connection on us
                        break 'asking;
                    }
                }
                for _ in 0..4 {
                    server.serve_pending(&mut session);
                }
                if server.connections.is_empty() {
                    dropped = true;
                    break;
                }
                if started.elapsed() > Duration::from_secs(60) {
                    break;
                }
            }
            assert!(dropped, "the server kept queueing replies for an agent that reads nothing");
            // Another agent is unaffected.
            let (mut other, mut lines) = connect(&file);
            writeln!(other, r#"{{"jsonrpc":"2.0","id":9,"method":"ping"}}"#).unwrap();
            assert_eq!(read_one(&mut lines, &mut server, &mut session)["id"], json!(9));
        }

        #[test]
        fn closing_one_instance_leaves_the_link_file_of_another() {
            let (file, _guard) = scratch("two");
            let first = LinkServer::start(&file, || {}).unwrap();
            let second = LinkServer::start(&file, || {}).unwrap();
            drop(first);
            assert!(file.exists(), "the second instance's file is still there");
            let info: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
            assert_eq!(info["port"].as_u64(), Some(u64::from(second.address().port())));
            drop(second);
            assert!(!file.exists(), "and goes with its owner");
        }

        #[test]
        fn hostile_handshakes_are_dropped() {
            let (file, _guard) = scratch("hostile");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            for junk in [vec![b'x'; 4096], b"\xff\xfe\n".to_vec(), b"[]\n".to_vec(), b"{\"token\":7}\n".to_vec(), b"\n".to_vec()] {
                let mut s = TcpStream::connect(server.address()).unwrap();
                let _ = s.write_all(&junk);
                let _ = s.write_all(b"\n");
                let mut rest = String::new();
                let _ = BufReader::new(s).read_to_string(&mut rest); // ends: the server hangs up
                assert!(!rest.contains("true"));
            }
            assert_eq!(server.serve_pending(&mut Session::new()), 0);
        }

        #[test]
        fn commands_that_would_discard_unsaved_work_are_refused_but_others_run() {
            let (file, _guard) = scratch("live");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            let mut session = Session::new();
            session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
            assert!(session.dirty);
            let (mut agent, mut lines) = connect(&file);
            writeln!(agent, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"file_new","arguments":{}}})).unwrap();
            let reply = pump(&mut server, &mut session, &mut lines);
            assert_eq!(reply["result"]["isError"], json!(true));
            assert_eq!(session.doc.shapes.len(), 1, "the person's drawing is still there");
        }

        #[test]
        fn a_subscribed_agent_hears_about_the_persons_own_edits() {
            let (file, _guard) = scratch("subscribe");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            let mut session = Session::new();
            let (mut agent, mut lines) = connect(&file);
            writeln!(agent, "{}", json!({"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":"laserfoundry://document/summary"}}))
                .unwrap();
            assert_eq!(read_one(&mut lines, &mut server, &mut session)["result"], json!({}));
            // Nothing changed, so nothing is announced.
            lines.get_ref().set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            server.serve_pending(&mut session);
            assert!(lines.read_line(&mut String::new()).is_err());
            // The person draws something in the window.
            session.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
            let note = read_one(&mut lines, &mut server, &mut session);
            assert_eq!(note["method"], json!("notifications/resources/updated"));
            assert_eq!(note["params"]["uri"], json!("laserfoundry://document/summary"));
        }

        #[test]
        fn the_link_file_is_private_and_removed_when_the_server_goes() {
            let (file, _guard) = scratch("file");
            let server = LinkServer::start(&file, || {}).unwrap();
            let info: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
            assert_eq!(info["port"], json!(server.address().port()));
            assert_eq!(info["token"].as_str().unwrap().len(), TOKEN_BYTES * 2);
            #[cfg(unix)]
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&fs::metadata(&file).unwrap().permissions()) & 0o077,
                0,
                "no access for group or others"
            );
            drop(server);
            assert!(!file.exists());
        }

        #[test]
        fn the_proxy_reports_a_missing_or_stale_link_file() {
            let (file, _guard) = scratch("stale");
            let err = proxy(&file, io::empty(), io::sink()).unwrap_err();
            assert!(err.to_string().contains("no running LaserFoundry"));
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "{\"port\": 1, \"token\": \"x\"}").unwrap();
            assert!(proxy(&file, io::empty(), io::sink()).is_err());
            fs::write(&file, "not json").unwrap();
            assert!(proxy(&file, io::empty(), io::sink()).is_err());
        }

        #[test]
        fn the_proxy_relays_a_session_end_to_end() {
            let (file, _guard) = scratch("proxy");
            let mut server = LinkServer::start(&file, || {}).unwrap();
            let done = Arc::new(AtomicBool::new(false));
            let flag = done.clone();
            let path = file.clone();
            let out = Arc::new(Mutex::new(Vec::new()));
            let sink = SharedSink(out.clone());
            let client = thread::spawn(move || {
                let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
                let r = proxy(&path, &input[..], sink);
                flag.store(true, Ordering::Release);
                r
            });
            let mut session = Session::new();
            while !done.load(Ordering::Acquire) {
                server.serve_pending(&mut session);
                thread::sleep(Duration::from_millis(5));
            }
            client.join().unwrap().unwrap();
            let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
            assert!(text.contains("\"id\":1"), "{text}");
        }
    }

    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().map_err(|_| io::Error::other("poisoned"))?.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Cleanup(PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> (PathBuf, Cleanup) {
        let dir = std::env::temp_dir().join(format!("lf-link-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (dir.join("agent-link.json"), Cleanup(dir))
    }

    /// Connects like an agent would, presenting the token from the link file.
    fn connect(file: &Path) -> (TcpStream, BufReader<TcpStream>) {
        let info: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, info["port"].as_u64().unwrap() as u16)).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        writeln!(writer, "{}", json!({ "token": info["token"] })).unwrap();
        let mut hello = String::new();
        reader.read_line(&mut hello).unwrap();
        assert!(hello.contains("true"), "{hello}");
        (writer, reader)
    }

    fn read_one(lines: &mut BufReader<TcpStream>, server: &mut LinkServer, session: &mut Session) -> Value {
        pump(server, session, lines)
    }

    /// Plays the app's frame loop until the agent has its answer.
    fn pump(server: &mut LinkServer, session: &mut Session, lines: &mut BufReader<TcpStream>) -> Value {
        lines.get_ref().set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut line = String::new();
        for _ in 0..500 {
            server.serve_pending(session);
            match lines.read_line(&mut line) {
                Ok(n) if n > 0 => return serde_json::from_str(&line).unwrap(),
                _ => {}
            }
        }
        panic!("no answer from the app");
    }
}
