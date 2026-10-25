//! Headless Language Server Protocol (LSP) Client Subsystem.
//!
//! Connects directly to language servers (`rust-analyzer`, `vtsls`, `sourcekit-lsp`)
//! via JSON-RPC 2.0 over stdio pipes, extracting compiler-verified diagnostics,
//! definitions, and hover types without reading thousands of raw lines.
//!
//! [`diagnose_files`] is the entry point the app uses: it starts the right
//! server for each changed file under the user's trust and sandbox
//! ([`crate::trust::Consent`]), opens the files, collects what the server
//! publishes and reports it in the same shape as [`crate::verify_loop`]. A
//! server that is not installed is skipped and said so, never an error.

use crate::trust::Consent;
use crate::verify_loop::{DiagnosticItem, VerificationResult};
use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// An LSP Diagnostic message received via publishDiagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub file_path: String,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    pub severity: u32, // 1=Error, 2=Warning, 3=Info, 4=Hint
    pub source: String,
    pub message: String,
}

/// JSON-RPC 2.0 Request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// Language Server Registry & Diagnostic Cache.
#[derive(Debug, Default)]
pub struct LspCache {
    /// file_path -> list of active diagnostics
    diagnostics_by_file: HashMap<String, Vec<LspDiagnostic>>,
}

impl LspCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_diagnostics(&mut self, file_path: &str, items: Vec<LspDiagnostic>) {
        self.diagnostics_by_file.insert(file_path.to_string(), items);
    }

    pub fn get_diagnostics(&self, file_path: &str) -> &[LspDiagnostic] {
        self.diagnostics_by_file
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn all_errors(&self) -> Vec<&LspDiagnostic> {
        self.diagnostics_by_file
            .values()
            .flat_map(|list| list.iter().filter(|d| d.severity == 1))
            .collect()
    }
}

pub struct HeadlessLsp;

impl HeadlessLsp {
    /// Build an LSP `textDocument/definition` request payload.
    pub fn make_definition_request(id: u64, uri: &str, line: u32, character: u32) -> JsonRpcRequest {
        JsonRpcRequest::new(
            id,
            "textDocument/definition",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )
    }

    /// Build an LSP `textDocument/hover` request payload.
    pub fn make_hover_request(id: u64, uri: &str, line: u32, character: u32) -> JsonRpcRequest {
        JsonRpcRequest::new(
            id,
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )
    }

    /// Parse an incoming `textDocument/publishDiagnostics` notification.
    pub fn parse_publish_diagnostics(val: &serde_json::Value) -> Result<(String, Vec<LspDiagnostic>)> {
        let uri = val["uri"].as_str().ok_or_else(|| ChorusError::Invalid("Missing uri".into()))?;
        let items = val["diagnostics"].as_array().ok_or_else(|| ChorusError::Invalid("Missing diagnostics".into()))?;

        let mut diags = Vec::new();
        for item in items {
            let range = &item["range"];
            let start = &range["start"];
            let end = &range["end"];

            diags.push(LspDiagnostic {
                file_path: uri.to_string(),
                line_start: start["line"].as_u64().unwrap_or(0) as u32,
                col_start: start["character"].as_u64().unwrap_or(0) as u32,
                line_end: end["line"].as_u64().unwrap_or(0) as u32,
                col_end: end["character"].as_u64().unwrap_or(0) as u32,
                severity: item["severity"].as_u64().unwrap_or(1) as u32,
                source: item["source"].as_str().unwrap_or("lsp").to_string(),
                message: item["message"].as_str().unwrap_or("").to_string(),
            });
        }

        Ok((uri.to_string(), diags))
    }
}

// ---- transport -----------------------------------------------------------

/// Write one framed JSON-RPC message (`Content-Length` header, then the body).
pub fn write_message(w: &mut impl Write, msg: &Value) -> std::io::Result<()> {
    let body = msg.to_string();
    write!(w, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    w.flush()
}

/// Read one framed message; `None` at end of stream or on a broken frame.
pub fn read_message(r: &mut impl BufRead) -> Option<Value> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().ok();
        }
    }
    let mut buf = vec![0u8; len?];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

/// `/a b/é.rs` → `file:///a%20b/%C3%A9.rs`.
pub fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn uri_to_path(uri: &str) -> PathBuf {
    let raw = uri.strip_prefix("file://").unwrap_or(uri).as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(&raw[i + 1..i + 3]), 16) {
                bytes.push(v);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

/// A language server we know how to start, chosen by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSpec {
    pub binary: &'static str,
    pub args: &'static [&'static str],
}

/// The server for `path`, and the LSP `languageId` to open it with.
pub fn server_for(path: &str) -> Option<(ServerSpec, &'static str)> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    let (binary, args, lang): (&'static str, &'static [&'static str], &'static str) = match ext.as_str() {
        "rs" => ("rust-analyzer", &[], "rust"),
        "ts" | "mts" | "cts" => ("typescript-language-server", &["--stdio"], "typescript"),
        "tsx" => ("typescript-language-server", &["--stdio"], "typescriptreact"),
        "js" | "mjs" | "cjs" => ("typescript-language-server", &["--stdio"], "javascript"),
        "jsx" => ("typescript-language-server", &["--stdio"], "javascriptreact"),
        "py" => ("pyright-langserver", &["--stdio"], "python"),
        "swift" => ("sourcekit-lsp", &[], "swift"),
        "go" => ("gopls", &[], "go"),
        _ => return None,
    };
    Some((ServerSpec { binary, args }, lang))
}

/// One running language server over stdio.
pub struct LspSession {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
}

impl LspSession {
    /// Start `cmd` (already confined by the caller), `initialize` it for
    /// `root` and wait for its answer.
    pub fn start(mut cmd: Command, root: &Path, init_timeout: Duration) -> Result<Self> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ChorusError::Invalid(format!("cannot start language server: {e}")))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Some(m) = read_message(&mut r) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        let mut s = Self { child, stdin, rx, next_id: 1 };
        let init = json!({
            "processId": std::process::id(),
            "rootUri": path_to_uri(root),
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": { "relatedInformation": false },
                    "synchronization": { "didSave": true }
                },
                "window": { "workDoneProgress": true }
            },
            "workspaceFolders": [{ "uri": path_to_uri(root), "name": "workspace" }],
        });
        let id = s.request("initialize", init)?;
        s.await_response(id, init_timeout)?;
        s.notify("initialized", json!({}))?;
        Ok(s)
    }

    fn send(&mut self, msg: Value) -> Result<()> {
        write_message(&mut self.stdin, &msg).map_err(|e| ChorusError::Invalid(format!("language server closed: {e}")))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(serde_json::to_value(JsonRpcRequest::new(id, method, params)).expect("serialisable"))?;
        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Answer a request the server made of us. We offer nothing, so the
    /// answer is always "null" (or one null per configuration item) — enough
    /// to keep servers that block on `window/workDoneProgress/create` or
    /// `workspace/configuration` moving.
    fn answer_server_request(&mut self, msg: &Value) {
        let (Some(id), Some(method)) = (msg.get("id"), msg["method"].as_str()) else { return };
        let result = match method {
            "workspace/configuration" => {
                Value::Array(vec![Value::Null; msg["params"]["items"].as_array().map_or(0, Vec::len)])
            }
            _ => Value::Null,
        };
        let _ = self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn await_response(&mut self, id: u64, timeout: Duration) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .rx
                .recv_timeout(left)
                .map_err(|_| ChorusError::Invalid("language server did not answer in time".into()))?;
            if msg.get("method").is_some() {
                self.answer_server_request(&msg);
            } else if msg["id"] == json!(id) {
                return Ok(msg);
            }
        }
    }

    /// Open `files` (absolute paths) and collect what the server publishes
    /// for them until every file has been reported at least once and the
    /// stream goes quiet, or `wait` runs out. Returns what arrived either way.
    pub fn diagnostics(&mut self, files: &[(PathBuf, &str)], wait: Duration) -> Vec<LspDiagnostic> {
        let mut pending: HashMap<String, ()> = HashMap::new();
        for (path, lang) in files {
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            let uri = path_to_uri(path);
            let item = json!({ "uri": uri, "languageId": lang, "version": 1, "text": text });
            if self.notify("textDocument/didOpen", json!({ "textDocument": item })).is_err() {
                return Vec::new();
            }
            // A save asks servers that check on save (rust-analyzer's cargo
            // check) to produce their full diagnostics, not just syntax ones.
            let _ = self.notify("textDocument/didSave", json!({ "textDocument": { "uri": uri } }));
            pending.insert(uri, ());
        }
        let mut latest: HashMap<String, Vec<LspDiagnostic>> = HashMap::new();
        let deadline = Instant::now() + wait;
        // Once every file has reported, give a slower second pass (cargo check
        // after syntax) this long to speak before settling.
        let settle = Duration::from_millis(1500);
        let mut quiet_since: Option<Instant> = None;
        while Instant::now() < deadline {
            let slice = if quiet_since.is_some() { settle } else { deadline.saturating_duration_since(Instant::now()) };
            match self.rx.recv_timeout(slice.min(deadline.saturating_duration_since(Instant::now()))) {
                Ok(msg) => {
                    if msg.get("method").is_some() && msg.get("id").is_some() {
                        self.answer_server_request(&msg);
                    } else if msg["method"] == "textDocument/publishDiagnostics" {
                        if let Ok((uri, items)) = HeadlessLsp::parse_publish_diagnostics(&msg["params"]) {
                            pending.remove(&uri);
                            latest.insert(uri, items);
                            quiet_since = pending.is_empty().then(Instant::now);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if quiet_since.is_some() => break,
                Err(_) => break,
            }
        }
        latest.into_values().flatten().collect()
    }

    /// Ask the server to stop; kill it if it does not.
    pub fn shutdown(mut self) {
        if let Ok(id) = self.request("shutdown", Value::Null) {
            let _ = self.await_response(id, Duration::from_secs(2));
            let _ = self.notify("exit", Value::Null);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LspSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// ---- the app-facing report ------------------------------------------------

fn severity_name(s: u32) -> &'static str {
    match s {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    }
}

fn to_item(d: &LspDiagnostic, root: &Path) -> DiagnosticItem {
    let abs = uri_to_path(&d.file_path);
    let file = abs.strip_prefix(root).unwrap_or(&abs).to_string_lossy().into_owned();
    DiagnosticItem {
        file,
        line: d.line_start as usize + 1,
        col: d.col_start as usize + 1,
        severity: severity_name(d.severity).to_string(),
        message: if d.source.is_empty() { d.message.clone() } else { format!("[{}] {}", d.source, d.message) },
    }
}

/// How long to wait for a server's diagnostics: `CHORUS_LSP_WAIT_SECS`, else 25 s
/// (rust-analyzer needs a while on a cold workspace).
pub fn default_wait() -> Duration {
    let secs = std::env::var("CHORUS_LSP_WAIT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(25u64);
    Duration::from_secs(secs.clamp(1, 300))
}

/// Run language-server diagnostics for `files` (paths relative to the
/// workspace, as `git diff --name-only` gives them). Only for a trusted
/// project and only inside its sandbox — a language server runs the
/// project's own build scripts and macros. Servers that are not installed are
/// listed in `raw_output`, not treated as failures.
pub fn diagnose_files(consent: &Consent, files: &[String], wait: Duration) -> Result<VerificationResult> {
    diagnose_with(consent, files, wait, &crate::trust::which)
}

/// [`diagnose_files`] with the way a server binary is found made explicit.
pub fn diagnose_with(
    consent: &Consent,
    files: &[String],
    wait: Duration,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Result<VerificationResult> {
    let start = Instant::now();
    let root = consent.workspace().to_path_buf();

    let mut by_server: Vec<(ServerSpec, Vec<(PathBuf, &'static str)>)> = Vec::new();
    for f in files {
        let Some((spec, lang)) = server_for(f) else { continue };
        let abs = root.join(f);
        if !abs.is_file() {
            continue; // deleted files have nothing to open
        }
        match by_server.iter_mut().find(|(s, _)| *s == spec) {
            Some((_, list)) => list.push((abs, lang)),
            None => by_server.push((spec, vec![(abs, lang)])),
        }
    }

    let mut items: Vec<DiagnosticItem> = Vec::new();
    let mut used: Vec<&str> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (spec, list) in by_server {
        let Some(bin) = resolve(spec.binary) else {
            notes.push(format!("{}: not installed — skipped {} file(s)", spec.binary, list.len()));
            continue;
        };
        let mut argv = vec![bin.to_string_lossy().into_owned()];
        argv.extend(spec.args.iter().map(|a| a.to_string()));
        let started = consent
            .command(&argv)
            .and_then(|cmd| LspSession::start(cmd, &root, Duration::from_secs(20)));
        let mut session = match started {
            Ok(s) => s,
            Err(e) => {
                notes.push(format!("{}: {e}", spec.binary));
                continue;
            }
        };
        let got = session.diagnostics(&list, wait);
        session.shutdown();
        used.push(spec.binary);
        items.extend(got.iter().map(|d| to_item(d, &root)));
    }
    items.sort_by(|a, b| (&a.file, a.line, a.col).cmp(&(&b.file, b.line, b.col)));
    items.dedup();

    let passed = !items.iter().any(|d| d.severity == "error");
    Ok(VerificationResult {
        passed,
        checker_used: if used.is_empty() { "lsp (no server)".to_string() } else { format!("lsp:{}", used.join("+")) },
        duration_ms: start.elapsed().as_millis() as u64,
        diagnostics: items,
        raw_output: notes.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_json_rpc_requests_format_correctly() {
        let req = HeadlessLsp::make_definition_request(42, "file:///src/main.rs", 10, 5);
        assert_eq!(req.id, 42);
        assert_eq!(req.method, "textDocument/definition");
        assert_eq!(req.params["textDocument"]["uri"], "file:///src/main.rs");
    }

    #[test]
    fn parse_publish_diagnostics_roundtrip() {
        let json = serde_json::json!({
            "uri": "file:///src/lib.rs",
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 5, "character": 4 },
                        "end": { "line": 5, "character": 12 }
                    },
                    "severity": 1,
                    "source": "rustc",
                    "message": "mismatched types"
                }
            ]
        });

        let (uri, diags) = HeadlessLsp::parse_publish_diagnostics(&json).unwrap();
        assert_eq!(uri, "file:///src/lib.rs");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "mismatched types");
        assert_eq!(diags[0].severity, 1);
    }

    #[test]
    fn messages_are_framed_and_read_back() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"jsonrpc":"2.0","method":"x","params":{"ไทย":1}})).unwrap();
        write_message(&mut buf, &json!({"id":2})).unwrap();
        let mut r = std::io::BufReader::new(&buf[..]);
        assert_eq!(read_message(&mut r).unwrap()["params"]["ไทย"], 1);
        assert_eq!(read_message(&mut r).unwrap()["id"], 2);
        assert!(read_message(&mut r).is_none());
    }

    #[test]
    fn uris_round_trip_through_spaces_and_thai() {
        for p in ["/tmp/a b/c.rs", "/Users/x/โปรเจกต์/main.rs", "/plain/path.ts"] {
            let u = path_to_uri(Path::new(p));
            assert!(u.starts_with("file:///"), "{u}");
            assert!(!u.contains(' '));
            assert_eq!(uri_to_path(&u), Path::new(p));
        }
    }

    #[test]
    fn servers_are_chosen_by_extension() {
        assert_eq!(server_for("src/lib.rs").unwrap().0.binary, "rust-analyzer");
        assert_eq!(server_for("a/b.tsx").unwrap(), (server_for("x.ts").unwrap().0, "typescriptreact"));
        assert_eq!(server_for("x.py").unwrap().0.binary, "pyright-langserver");
        assert!(server_for("README.md").is_none());
        assert!(server_for("Makefile").is_none());
    }

    /// A minimal language server in Python: answers `initialize`, then reports
    /// one error and one warning for every file it is told is opened.
    const FAKE_SERVER: &str = r#"
import sys, json
def read():
    n = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line: return None
        line = line.decode().strip()
        if not line: break
        if line.lower().startswith("content-length:"): n = int(line.split(":")[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b); sys.stdout.buffer.flush()
while True:
    m = read()
    if m is None: break
    meth = m.get("method")
    if meth == "initialize":
        # Ask the client something first: it must answer, or this blocks.
        send({"jsonrpc":"2.0","id":900,"method":"window/workDoneProgress/create","params":{"token":"t"}})
        send({"jsonrpc":"2.0","id":m["id"],"result":{"capabilities":{}}})
    elif meth == "textDocument/didOpen":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[
          {"range":{"start":{"line":2,"character":4},"end":{"line":2,"character":9}},"severity":1,"source":"fake","message":"mismatched types"},
          {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"severity":2,"message":"unused import"}]}})
    elif meth == "shutdown":
        send({"jsonrpc":"2.0","id":m["id"],"result":None})
    elif meth == "exit":
        break
"#;

    fn fake_server_script(dir: &Path) -> PathBuf {
        let py = dir.join("fake_lsp.py");
        std::fs::write(&py, FAKE_SERVER).unwrap();
        let sh = dir.join("fake-lsp");
        std::fs::write(&sh, format!("#!/bin/sh\nexec python3 \"{}\"\n", py.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        sh
    }

    fn python_available() -> bool {
        Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn a_session_collects_published_diagnostics() {
        if !python_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "use x;\n\n    let a = 1;\n").unwrap();
        let sh = fake_server_script(dir.path());
        let mut s = LspSession::start(Command::new(&sh), dir.path(), Duration::from_secs(10)).unwrap();
        let got = s.diagnostics(&[(file.clone(), "rust")], Duration::from_secs(10));
        s.shutdown();
        assert_eq!(got.len(), 2, "{got:?}");
        let err = got.iter().find(|d| d.severity == 1).unwrap();
        assert_eq!((err.line_start, err.col_start, err.message.as_str()), (2, 4, "mismatched types"));
        assert_eq!(uri_to_path(&err.file_path), file);
    }

    fn consent_for<'a>(ws: &Path, home: &Path, confine: &'a crate::trust::Confine<'a>) -> Consent<'a> {
        let store = crate::trust::TrustStore::at(home.join("t.json"));
        store.set_trusted(ws, true).unwrap();
        store.consent(ws, confine).unwrap()
    }

    #[test]
    fn diagnose_files_reports_in_the_verification_shape() {
        if !python_available() {
            return;
        }
        let ws = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ws.path().join("src")).unwrap();
        std::fs::write(ws.path().join("src/lib.rs"), "use x;\n\n    let a = 1;\n").unwrap();
        std::fs::write(ws.path().join("notes.md"), "no server for this").unwrap();
        let sh = fake_server_script(home.path());
        let identity = |argv: &[String], _: &Path| -> std::result::Result<Vec<String>, String> { Ok(argv.to_vec()) };
        let consent = consent_for(ws.path(), home.path(), &identity);
        let files = vec!["src/lib.rs".to_string(), "notes.md".to_string(), "gone.rs".to_string()];
        let resolve = |b: &str| (b == "rust-analyzer").then(|| sh.clone());
        let r = diagnose_with(&consent, &files, Duration::from_secs(10), &resolve).unwrap();
        assert!(!r.passed, "an error was reported");
        assert_eq!(r.checker_used, "lsp:rust-analyzer");
        let first = &r.diagnostics[0];
        // Relative path, 1-based line/column — the shape the review panel already shows.
        assert_eq!((first.file.as_str(), first.line, first.col), ("src/lib.rs", 1, 1));
        let e = r.diagnostics.iter().find(|d| d.severity == "error").unwrap();
        assert_eq!((e.line, e.col, e.message.as_str()), (3, 5, "[fake] mismatched types"));
    }

    #[test]
    fn a_missing_server_is_a_note_not_an_error() {
        let ws = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("a.rs"), "fn main() {}\n").unwrap();
        let identity = |argv: &[String], _: &Path| -> std::result::Result<Vec<String>, String> { Ok(argv.to_vec()) };
        let consent = consent_for(ws.path(), home.path(), &identity);
        let none = |_: &str| None;
        let r = diagnose_with(&consent, &["a.rs".to_string()], Duration::from_secs(1), &none).unwrap();
        assert!(r.passed);
        assert!(r.diagnostics.is_empty());
        assert_eq!(r.checker_used, "lsp (no server)");
        assert!(r.raw_output.contains("rust-analyzer: not installed"), "{}", r.raw_output);
    }

    #[test]
    fn an_untrusted_project_gets_no_language_server() {
        let ws = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let store = crate::trust::TrustStore::at(home.path().join("t.json"));
        let identity = |argv: &[String], _: &Path| -> std::result::Result<Vec<String>, String> { Ok(argv.to_vec()) };
        assert!(store.consent(ws.path(), &identity).is_err(), "no consent, no way to call diagnose_files");
    }
}
