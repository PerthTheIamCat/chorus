//! A deliberately dumb ACP client: enough JSON-RPC to hold a conversation with
//! an agent process, and no interpretation of what it says.
//!
//! This exists to answer three questions before we design the adapter layer on
//! top of assumptions:
//!
//!   1. Does `session/load` actually give us a session back, and what comes with it?
//!   2. How much of the permission flow do we control from the client side?
//!   3. Do diffs and terminals arrive as structured updates, or as prose?
//!
//! Transport is newline-delimited JSON over the agent's stdin/stdout — one JSON
//! object per line, no Content-Length headers.

pub mod codex;
pub mod env;
pub mod registry;
pub mod sandbox;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Every frame in both directions is appended here, verbatim. The transcript is
/// the actual deliverable of this spike — the summary is just a reading of it.
pub type Log = Arc<Mutex<Box<dyn Write + Send>>>;

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn log_frame(log: &Log, dir: &str, frame: &Value) {
    let line = json!({ "t": now_ms(), "dir": dir, "frame": frame });
    if let Ok(mut w) = log.lock() {
        let _ = writeln!(w, "{line}");
        let _ = w.flush();
    }
}

fn log_raw(log: &Log, dir: &str, text: &str) {
    let line = json!({ "t": now_ms(), "dir": dir, "raw": text });
    if let Ok(mut w) = log.lock() {
        let _ = writeln!(w, "{line}");
        let _ = w.flush();
    }
}

/// Pid of the agent process currently answering a turn (0 = none).
///
/// Process-global on purpose: the shell's Stop button has no handle to the
/// `Agent` (it lives on the turn thread, blocked in `prompt`), and only one
/// turn runs at a time. Killing the process unblocks `prompt` with an error;
/// [`was_cancelled`] is how the caller tells that apart from a real failure.
static ACTIVE_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called with `(context_used, context_window)` on every `usage_update`, on
/// the thread running the turn. Thread-local, unlike [`PushSink`]: the
/// session that owns the turn installs it around its own `prompt` call to
/// persist the numbers, and must not see another thread's turn.
pub type UsageObserver = Box<dyn FnMut(Option<i64>, Option<i64>)>;

thread_local! {
    static USAGE: std::cell::RefCell<Option<UsageObserver>> = const { std::cell::RefCell::new(None) };
}

/// Install (or clear) this thread's usage observer; returns the previous one.
pub fn set_usage_observer(observer: Option<UsageObserver>) -> Option<UsageObserver> {
    USAGE.with(|u| std::mem::replace(&mut *u.borrow_mut(), observer))
}

pub(crate) fn observe_usage(used: Option<i64>, size: Option<i64>) {
    USAGE.with(|u| {
        if let Some(f) = u.borrow_mut().as_mut() {
            f(used, size);
        }
    });
}

/// Live signals from a running turn, tagged with the session they belong to.
pub use push::{emit_progress, push, push_scope, set_push_sink, Push, PushScope, PushSink};

pub mod push {
    //! The one way out of a running turn while it holds its engine queue.
    //!
    //! Every signal names its `session_id` and the newest ledger `seq` that
    //! session is known to have, so a shell watching several sessions (Orchestra
    //! runs children side by side) can route each one, and can tell from `seq`
    //! alone whether it has fallen behind the ledger. Signals are hints, never
    //! the record: a dropped one is recovered by the next delta pull, and
    //! nothing here is written anywhere.
    //!
    //! Kinds: `thought` · `message` · `tool` · `tool_done` (`status\ttitle`) ·
    //! `usage` (`used\tsize`) · `events` (new rows up to `seq`) · `status`
    //! (`running` | `idle` | `failed`).

    use std::cell::Cell;
    use std::sync::Mutex;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Push<'a> {
        pub session_id: i64,
        pub seq: i64,
        pub kind: &'a str,
        pub text: &'a str,
    }

    /// Process-global: there is one shell, and it wants every session's
    /// signals (sidebar badges need background ones too). Called on whatever
    /// thread the turn runs on.
    pub type PushSink = Box<dyn Fn(&Push) + Send + Sync>;
    static SINK: Mutex<Option<PushSink>> = Mutex::new(None);

    thread_local! {
        /// `(session_id, seq)` of the turn this thread is running. Thread-local
        /// so concurrent turns on different threads never tag each other's
        /// tokens — the ACP reader has no idea which Chorus session it serves.
        static SCOPE: Cell<Option<(i64, i64)>> = const { Cell::new(None) };
    }

    pub fn set_push_sink(sink: Option<PushSink>) {
        if let Ok(mut s) = SINK.lock() {
            *s = sink;
        }
    }

    /// While the guard lives, [`emit_progress`] on this thread is tagged with
    /// `session_id`. Restores the previous tag on drop, so nesting is safe.
    #[must_use = "the scope ends when the guard is dropped"]
    pub struct PushScope {
        prev: Option<(i64, i64)>,
    }

    pub fn push_scope(session_id: i64, seq: i64) -> PushScope {
        PushScope {
            prev: SCOPE.with(|s| s.replace(Some((session_id, seq)))),
        }
    }

    impl Drop for PushScope {
        fn drop(&mut self) {
            SCOPE.with(|s| s.set(self.prev));
        }
    }

    /// Send one signal for an explicit session. Also moves this thread's
    /// scope forward when it is the same session, so later tokens carry the
    /// newer `seq`.
    pub fn push(session_id: i64, seq: i64, kind: &str, text: &str) {
        SCOPE.with(|s| {
            if let Some((sid, old)) = s.get() {
                if sid == session_id && seq > old {
                    s.set(Some((sid, seq)));
                }
            }
        });
        deliver(&Push {
            session_id,
            seq,
            kind,
            text,
        });
    }

    /// Send one signal for the session this thread is running. Outside a
    /// [`push_scope`] there is no session to name, so nothing is sent.
    pub fn emit_progress(kind: &str, text: &str) {
        if let Some((session_id, seq)) = SCOPE.with(Cell::get) {
            deliver(&Push {
                session_id,
                seq,
                kind,
                text,
            });
        }
    }

    fn deliver(p: &Push) {
        if let Ok(s) = SINK.lock() {
            if let Some(f) = s.as_ref() {
                f(p);
            }
        }
    }
}

/// Stop the running turn and any children. Returns false when no agent is running.
pub fn cancel_active() -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    let pid = ACTIVE_PID.load(SeqCst);
    CANCELLED.store(true, SeqCst);
    let list = registry::list();
    for s in &list {
        kill_group(s.pid, 15);
    }
    if pid != 0 {
        kill_group(pid, 15);
        true
    } else {
        !list.is_empty()
    }
}

extern "C" {
    fn killpg(pgrp: i32, sig: i32) -> i32;
}

/// Signal the process group led by `pid`. Agents are spawned as group leaders,
/// so this takes out grandchildren (cli.js, MCP servers, language servers) that
/// killing the direct child alone would orphan to launchd.
fn kill_group(pid: u32, sig: i32) {
    unsafe {
        killpg(pid as i32, sig);
    }
}

/// Kill all spawned agent process groups tracked by the registry and mark cancelled.
pub fn cancel_all() -> usize {
    CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
    let pid = ACTIVE_PID.load(std::sync::atomic::Ordering::SeqCst);
    if pid != 0 {
        kill_group(pid, 15);
    }
    let list = registry::list();
    for s in &list {
        kill_group(s.pid, 15);
    }
    list.len()
}

/// Kill the running agent and its whole process tree. Call when the app quits:
/// `Drop` never runs when the host process exits without unwinding.
pub fn shutdown() {
    let pid = ACTIVE_PID.swap(0, std::sync::atomic::Ordering::SeqCst);
    if pid != 0 {
        kill_group(pid, 9);
    }
}

/// True while an agent is answering a turn. The system monitor refuses
/// `VACUUM` then: it would wait on, or stall, the turn's own writes.
pub fn turn_running() -> bool {
    ACTIVE_PID.load(std::sync::atomic::Ordering::SeqCst) != 0
}

/// True if the last turn ended because [`cancel_active`] killed it.
pub fn was_cancelled() -> bool {
    CANCELLED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Call before starting a turn.
pub fn reset_cancel() {
    CANCELLED.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// How long an agent may say nothing at all before a turn is treated as hung.
/// Long builds that stream nothing are the reason this is generous; override
/// with `CHORUS_IDLE_TIMEOUT_S` (0 = off, falls back to the hard timeout).
fn idle_timeout() -> Duration {
    match std::env::var("CHORUS_IDLE_TIMEOUT_S")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(0) => Duration::from_secs(u64::MAX / 4),
        Some(n) => Duration::from_secs(n),
        None => Duration::from_secs(240),
    }
}

/// A spawned agent process we speak ACP to.
pub struct Agent {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    rx: Receiver<Value>,
    next_id: AtomicI64,
    log: Log,
    /// The last few stderr lines. When an agent dies before saying anything,
    /// this is the only thing that explains why — so it belongs in the error,
    /// not just in the transcript.
    stderr_tail: Arc<Mutex<Vec<String>>>,
}

/// Anything the agent sent us that wasn't the response we were waiting for.
pub enum Inbound {
    /// A notification, e.g. `session/update`.
    Notification { method: String, params: Value },
    /// A request we are expected to answer, e.g. `session/request_permission`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

impl Agent {
    pub fn spawn(cmd: &[String], log: Log) -> Result<Self> {
        Self::spawn_env(cmd, &[], log)
    }

    /// Like [`spawn`], with extra environment variables — how an account is
    /// chosen (`CODEX_HOME` / `CLAUDE_CONFIG_DIR` pointing at its own dir).
    pub fn spawn_env(cmd: &[String], env: &[(String, String)], log: Log) -> Result<Self> {
        let (program, args) = cmd.split_first().ok_or_else(|| anyhow!("empty command"))?;
        let mut command = Command::new(program);
        command.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        // A Finder-launched app has launchd's bare PATH; without the user's
        // real one the agent finds Apple's git but not gh, node or npx.
        let explicit = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| std::ffi::OsString::from(v));
        command.env("PATH", env::agent_path(explicit.as_ref()));
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Chorus is the session host, not a nested agent. If we were
        // launched from inside another agent's CLI (Claude Code sets
        // `CLAUDECODE`), strip its markers so a spawned `claude` bridge
        // doesn't refuse to run "inside another Claude Code session".
        for var in [
            "CLAUDECODE",
            "CLAUDE_CODE_SSE_PORT",
            "CLAUDE_CODE_ENTRYPOINT",
        ] {
            command.env_remove(var);
        }
        // Own process group, so teardown can signal the whole tree.
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().with_context(|| format!("spawn {cmd:?}"))?;
        ACTIVE_PID.store(child.id(), std::sync::atomic::Ordering::SeqCst);
        registry::register(child.id(), registry::label_for(cmd), true);

        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let stdin = child.stdin.take().expect("piped");
        let (tx, rx) = channel();

        let rlog = log.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        log_frame(&rlog, "in", &v);
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                    // An agent that prints a banner to stdout is a real thing
                    // that happens; record it rather than dying on it.
                    Err(_) => log_raw(&rlog, "in-unparsed", &line),
                }
            }
        });

        let elog = log.clone();
        let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let etail = tail.clone();
        thread::spawn(move || {
            for l in BufReader::new(stderr)
                .lines()
                .map_while(std::result::Result::ok)
            {
                log_raw(&elog, "stderr", &l);
                if let Ok(mut t) = etail.lock() {
                    t.push(l);
                    if t.len() > 20 {
                        t.remove(0);
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            rx,
            next_id: AtomicI64::new(1),
            log,
            stderr_tail: tail,
        })
    }

    fn send(&self, v: Value) -> Result<()> {
        log_frame(&self.log, "out", &v);
        let line = serde_json::to_string(&v)?;
        let mut w = self.stdin.lock().map_err(|_| anyhow!("stdin poisoned"))?;
        // A child that already died gives us EPIPE here. Say that plainly
        // instead of letting a raw io error surface as the whole story.
        let mut write = || -> std::io::Result<()> {
            w.write_all(line.as_bytes())?;
            w.write_all(b"\n")?;
            w.flush()
        };
        match write() {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                Err(anyhow!("agent ปิด stdin ไปแล้ว (process น่าจะตายก่อนหน้านี้)"))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn request(&self, method: &str, params: Value) -> Result<i64> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        Ok(id)
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }

    pub fn respond(&self, id: &Value, result: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }

    pub fn respond_err(&self, id: &Value, code: i64, message: &str) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
    }

    /// Pump frames until the response to `id` arrives, handing everything else
    /// to `on_inbound`. Anything the agent asks us must be answered inside that
    /// callback, or the agent will sit there waiting and we'll time out.
    pub fn wait_for(
        &self,
        id: i64,
        timeout: Duration,
        on_inbound: &mut dyn FnMut(&Agent, Inbound),
    ) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        let idle = idle_timeout();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(anyhow!("timed out waiting for response to request {id}"));
            }
            // Every frame (message chunk, tool update, even a heartbeat)
            // resets this clock, so only a *silent* agent trips it. The hard
            // `timeout` above still bounds the whole call.
            let wait = left.min(idle);
            let v = match self.rx.recv_timeout(wait) {
                Ok(v) => v,
                Err(RecvTimeoutError::Timeout) if wait < left => {
                    return Err(anyhow!(
                        "agent stalled: no output for {}s while waiting for request {id}",
                        idle.as_secs()
                    ))
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(anyhow!("timed out waiting for response to request {id}"))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!(
                        "agent closed its output before answering request {id}"
                    ))
                }
            };

            let has_method = v.get("method").and_then(Value::as_str).is_some();
            match (v.get("id"), has_method) {
                // Response to us.
                (Some(rid), false) if rid.as_i64() == Some(id) => {
                    if let Some(err) = v.get("error") {
                        return Err(anyhow!("agent returned error: {err}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                // Response to some other request of ours — shouldn't happen in
                // this scripted probe, but don't lose it silently.
                (Some(_), false) => log_raw(&self.log, "note", "response with unexpected id"),
                // Request from the agent.
                (Some(rid), true) => on_inbound(
                    self,
                    Inbound::Request {
                        id: rid.clone(),
                        method: v["method"].as_str().unwrap_or_default().to_string(),
                        params: v.get("params").cloned().unwrap_or(Value::Null),
                    },
                ),
                // Notification.
                (None, true) => on_inbound(
                    self,
                    Inbound::Notification {
                        method: v["method"].as_str().unwrap_or_default().to_string(),
                        params: v.get("params").cloned().unwrap_or(Value::Null),
                    },
                ),
                (None, false) => log_raw(&self.log, "note", "frame with neither id nor method"),
            }
        }
    }

    /// Drain whatever is still queued, for the window after a call returns.
    pub fn drain(&self, quiet_for: Duration, on_inbound: &mut dyn FnMut(&Agent, Inbound)) {
        while let Ok(v) = self.rx.recv_timeout(quiet_for) {
            let has_method = v.get("method").and_then(Value::as_str).is_some();
            if !has_method {
                continue;
            }
            let method = v["method"].as_str().unwrap_or_default().to_string();
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            match v.get("id") {
                Some(id) => on_inbound(
                    self,
                    Inbound::Request {
                        id: id.clone(),
                        method,
                        params,
                    },
                ),
                None => on_inbound(self, Inbound::Notification { method, params }),
            }
        }
    }

    /// What the agent printed to stderr, most recent last.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default()
    }

    /// `Some(status)` if the process has already exited.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Why the conversation failed, in the terms a person can act on: the
    /// process died, or it is alive but not talking.
    pub fn diagnose(&mut self, err: &anyhow::Error) -> String {
        let mut s = format!("{err}");
        // Give a just-exited process a moment to be reaped.
        for _ in 0..10 {
            if self.exited().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        match self.exited() {
            Some(st) => s.push_str(&format!("\n  process จบไปแล้ว ({st})")),
            None => s.push_str("\n  process ยังอยู่แต่ไม่ตอบ"),
        }
        let tail = self.stderr_tail();
        if tail.is_empty() {
            s.push_str("\n  ไม่มีอะไรใน stderr เลย");
        } else {
            s.push_str("\n  stderr:");
            for l in tail {
                s.push_str(&format!("\n    {l}"));
            }
        }
        s
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = ACTIVE_PID.compare_exchange(
            self.child.id(),
            0,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        );
        kill_group(self.child.id(), 9);
        let _ = self.child.kill();
        let _ = self.child.wait();
        registry::unregister(self.child.id());
    }
}

/// A fake ACP agent, so the probe can be exercised without a real CLI, an API
/// key, or a network. It answers the methods the probe scripts and, in the
/// middle of a prompt, asks the client for permission — which is the part of the
/// client most likely to deadlock if we got it wrong.
pub mod mock {
    use anyhow::Result;
    use serde_json::{json, Value};
    use std::io::{BufRead, Write};

    fn send(v: Value) -> Result<()> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{}", serde_json::to_string(&v)?)?;
        out.flush()?;
        Ok(())
    }

    fn update(session: &str, update: Value) -> Result<()> {
        send(json!({
            "jsonrpc":"2.0",
            "method":"session/update",
            "params": { "sessionId": session, "update": update }
        }))
    }

    /// The `mode` entry of an ACP session config option list, in the shape the
    /// spec requires a `session/set_config_option` reply to carry.
    fn config_option(current: &str) -> Value {
        json!({
            "id": "mode",
            "name": "Mode",
            "category": "mode",
            "type": "select",
            "currentValue": current,
            "options": [
                {"name":"Ask for approval","value":"read-only"},
                {"name":"Approve for me","value":"agent"},
                {"name":"Full access","value":"agent-full-access"}
            ]
        })
    }

    pub fn serve() -> Result<()> {
        let stdin = std::io::stdin();
        let mut lines = stdin.lock().lines();
        let session = "mock-session-1";
        // The cwd the client opened the session with. A mock that reports paths
        // outside the workspace makes every downstream artefact (briefs, file
        // lists, the demo) look wrong for reasons that have nothing to do with
        // the code under test.
        let mut cwd = String::from("/tmp/mock");
        let mut mode = String::from("agent");
        let mut next_out_id = 1000;

        // `--serve-mock --fail <text>` makes every `session/prompt` return a
        // JSON-RPC error carrying `<text>` — lets the relay-chain test drive a
        // simulated rate limit without a network.
        let fail: Option<String> = {
            let a: Vec<String> = std::env::args().collect();
            a.iter()
                .position(|x| x == "--fail")
                .and_then(|i| a.get(i + 1).cloned())
        };

        // `--serve-mock --rogue <name>` writes `<cwd>/<name>` during the turn
        // without asking anyone. It exists to stand in for what Codex actually
        // does: accept `read-only`, report it back, then write files with no
        // `session/request_permission` at all. The client's guard is what has to
        // catch that, so the guard needs something real to catch.
        let rogue: Option<String> = {
            let a: Vec<String> = std::env::args().collect();
            a.iter()
                .position(|x| x == "--rogue")
                .and_then(|i| a.get(i + 1).cloned())
        };

        while let Some(Ok(line)) = lines.next() {
            if line.trim().is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let id = msg.get("id").cloned();
            let method = msg
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            match method.as_str() {
                "initialize" => send(json!({"jsonrpc":"2.0","id":id,"result":{
                    "protocolVersion": 1,
                    "agentCapabilities": {
                        "loadSession": true,
                        "promptCapabilities": {"image": false, "audio": false, "embeddedContext": true}
                    },
                    "agentInfo": {"name":"mock-agent","version":"0.1.0"},
                    "authMethods": []
                }}))?,

                "session/new" => {
                    if let Some(c) = msg["params"]["cwd"].as_str() {
                        cwd = c.to_string();
                    }
                    send(json!({"jsonrpc":"2.0","id":id,"result":{
                    "sessionId": session,
                    "modes": {"currentModeId": mode, "availableModes":[
                        {"id":"read-only","name":"Ask for approval"},
                        {"id":"agent","name":"Approve for me"},
                        {"id":"agent-full-access","name":"Full access"}
                    ]},
                    "configOptions": [config_option(&mode)]
                    }}))?
                }

                // A well-behaved agent: it takes the mode and reports it back, so
                // the client can tell an accepted restriction from an ignored one.
                // (Codex accepts and reports but does not obey — that is the real
                // bridge's problem, and the client's guard is what catches it.)
                "session/set_config_option" => {
                    if msg["params"]["configId"] == "mode" {
                        if let Some(v) = msg["params"]["value"].as_str() {
                            mode = v.to_string();
                        }
                    }
                    send(json!({"jsonrpc":"2.0","id":id,
                    "result":{"configOptions":[config_option(&mode)]}}))?
                }

                "session/set_mode" => {
                    if let Some(v) = msg["params"]["modeId"].as_str() {
                        mode = v.to_string();
                    }
                    send(json!({"jsonrpc":"2.0","id":id,"result":{}}))?
                }

                "session/prompt" if fail.is_some() => {
                    send(json!({"jsonrpc":"2.0","id":id,
                    "error":{"code":-32010,"message": fail.clone().unwrap()}}))?;
                }

                "session/prompt" => {
                    if let Some(name) = &rogue {
                        let _ = std::fs::write(std::path::Path::new(&cwd).join(name), "pwned\n");
                    }
                    update(
                        session,
                        json!({
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"type":"text","text":"กำลังดูไฟล์ให้ครับ"}
                        }),
                    )?;
                    // Context pressure mid-turn, the way both real bridges report it.
                    update(
                        session,
                        json!({
                            "sessionUpdate": "usage_update", "used": 12_000, "size": 200_000
                        }),
                    )?;
                    update(
                        session,
                        json!({
                            "sessionUpdate": "tool_call",
                            "toolCallId": "t1",
                            "title": format!("Read {cwd}/README.md"),
                            "kind": "read",
                            "status": "pending"
                        }),
                    )?;

                    // Ask the client for permission and block until it answers.
                    let req_id = next_out_id;
                    next_out_id += 1;
                    send(json!({
                        "jsonrpc":"2.0","id":req_id,"method":"session/request_permission",
                        "params": {
                            "sessionId": session,
                            "toolCall": {"toolCallId":"t1","title":format!("Read {cwd}/README.md"),"kind":"read"},
                            "options": [
                                {"optionId":"allow-once","name":"Allow once","kind":"allow_once"},
                                {"optionId":"allow-always","name":"Always allow","kind":"allow_always"},
                                {"optionId":"reject-once","name":"Reject","kind":"reject_once"}
                            ]
                        }
                    }))?;

                    let mut chosen = String::from("<none>");
                    for l in lines.by_ref() {
                        let Ok(l) = l else { break };
                        let Ok(v) = serde_json::from_str::<Value>(&l) else {
                            continue;
                        };
                        if v.get("id").and_then(Value::as_i64) == Some(req_id) {
                            chosen = v["result"]["outcome"]["optionId"]
                                .as_str()
                                .unwrap_or("<cancelled>")
                                .to_string();
                            break;
                        }
                    }

                    update(
                        session,
                        json!({
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": "t1",
                            "status": if chosen.starts_with("allow") { "completed" } else { "failed" },
                            "content": [{"type":"diff","path":format!("{cwd}/README.md"),"oldText":"# old","newText":"# new"}]
                        }),
                    )?;
                    update(
                        session,
                        json!({
                            "sessionUpdate": "plan",
                            "entries": [{"content":"อ่านไฟล์","priority":"high","status":"completed"}]
                        }),
                    )?;
                    update(
                        session,
                        json!({
                            "sessionUpdate": "usage_update", "used": 14_500, "size": 200_000,
                            "cost": {"amount": 0.0125, "currency": "USD"}
                        }),
                    )?;

                    send(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn"}}))?;
                }

                "session/load" => {
                    // Replay history, then answer. This is exactly the behaviour the
                    // probe is trying to detect in a real agent.
                    update(
                        session,
                        json!({
                            "sessionUpdate": "user_message_chunk",
                            "content": {"type":"text","text":"(replay) คำถามเดิม"}
                        }),
                    )?;
                    update(
                        session,
                        json!({
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"type":"text","text":"(replay) คำตอบเดิม"}
                        }),
                    )?;
                    send(json!({"jsonrpc":"2.0","id":id,"result":{}}))?;
                }

                _ if id.is_some() => send(json!({"jsonrpc":"2.0","id":id,
                "error":{"code":-32601,"message":"mock agent: no such method"}}))?,

                _ => {}
            }
        }
        Ok(())
    }
}
