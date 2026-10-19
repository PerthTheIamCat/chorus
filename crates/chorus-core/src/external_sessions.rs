//! Conversations other tools left on this machine — Claude Code, Codex,
//! Gemini CLI — found on disk and imported into the ledger, so a session
//! started in one of those apps continues here with any agent. Once imported
//! the ledger owns it (design rule 2.2): nothing here keeps a provider
//! session alive or writes back to the provider's files.
//!
//! On-disk formats (checked 19 Oct 2026 against each tool's docs/source):
//! - **Claude Code** `<config>/projects/<encoded-cwd>/<session-id>.jsonl`.
//!   Lines are `type: user|assistant|summary|…` with `message.content` as a
//!   string or content blocks. Anthropic documents the entry format as
//!   internal and version-dependent, so every field is treated as optional.
//! - **Codex** `<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-*.jsonl` or
//!   `.jsonl.zst`. Lines are `{timestamp, type, payload}`; the first is
//!   `session_meta` (codex-rs/rollout).
//! - **Gemini CLI** `~/.gemini/tmp/<project_hash>/chats/*.jsonl` (legacy
//!   `.json` = one whole record). A metadata line, then message records
//!   `{id, type: user|gemini|…, content, toolCalls}`, plus `$rewindTo`,
//!   `$set` and `$patch` records that edit earlier ones.
//!
//! These files are the user's own transcripts and can hold anything they
//! pasted, so nothing here logs their content.

use crate::types::INLINE_MAX_BYTES;
use crate::{actor, ledger, ChorusError, EventKind, NewEvent, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Lines read from the top of a file to find its id, cwd and title.
const PEEK_LINES: usize = 400;
/// Files above this are skipped by content search (still listed and importable).
const SEARCH_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Files scanned per search, newest first.
const SEARCH_MAX_FILES: usize = 2000;
/// A legacy Gemini `.json` is parsed whole; refuse anything absurd.
const LEGACY_JSON_MAX_BYTES: u64 = 32 * 1024 * 1024;
/// Per-event caps. Tool output is what bloats a ledger (CLAUDE.md 7.4).
const TOOL_MAX_CHARS: usize = 4_000;
const TEXT_MAX_CHARS: usize = 32_000;
const TITLE_MAX_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    Gemini,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Gemini];

    /// The agent name Chorus already uses for this provider's CLI.
    pub fn agent(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
        }
    }

    pub fn vendor(self) -> &'static str {
        match self {
            Self::Claude => "anthropic",
            Self::Codex => "openai",
            Self::Gemini => "google",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" | "gemini-cli" => Some(Self::Gemini),
            _ => None,
        }
    }
}

/// One provider config dir to look in: `~/.claude`, a `CODEX_HOME`, an extra
/// Chorus account dir.
#[derive(Debug, Clone)]
pub struct Root {
    pub provider: Provider,
    pub dir: PathBuf,
    /// The Chorus account name when `dir` is an extra account (`work` for
    /// `claude@work`); `None` for the CLI's own default dir.
    pub account: Option<String>,
}

/// The default config dir of each provider, honouring the same env vars the
/// CLIs do. Extra accounts are added by the caller, which knows about them.
pub fn default_roots() -> Vec<Root> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let env_or = |var: &str, rel: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(rel)))
    };
    let mut out = Vec::new();
    for (provider, dir) in [
        (Provider::Claude, env_or("CLAUDE_CONFIG_DIR", ".claude")),
        (Provider::Codex, env_or("CODEX_HOME", ".codex")),
        (Provider::Gemini, home.as_ref().map(|h| h.join(".gemini"))),
    ] {
        if let Some(dir) = dir {
            out.push(Root { provider, dir, account: None });
        }
    }
    out
}

/// A conversation found on disk. Cheap to build: only the head of the file
/// is read.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalSession {
    pub provider: Provider,
    pub account: Option<String>,
    pub id: String,
    pub path: PathBuf,
    /// The working directory the session ran in, when the file records one.
    pub cwd: Option<String>,
    /// First user message, or the tool's own summary/title.
    pub title: String,
    pub updated_ms: i64,
    pub size_bytes: u64,
    /// Set by [`search`] when the match was inside the conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

impl ExternalSession {
    /// The agent name this session's replies are recorded under.
    pub fn agent_name(&self) -> String {
        match &self.account {
            Some(a) => format!("{}@{a}", self.provider.agent()),
            None => self.provider.agent().to_string(),
        }
    }

    /// Stable key for "was this already imported".
    pub fn source_key(&self) -> String {
        format!("{}:{}", self.provider.agent(), self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    User,
    Agent,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalTurn {
    pub role: TurnRole,
    pub text: String,
}

// ── discovery ────────────────────────────────────────────────────────────

struct Candidate {
    provider: Provider,
    account: Option<String>,
    path: PathBuf,
    updated_ms: i64,
    size_bytes: u64,
}

fn candidates(roots: &[Root]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for root in roots {
        let files = match root.provider {
            Provider::Claude => claude_files(&root.dir),
            Provider::Codex => codex_files(&root.dir),
            Provider::Gemini => gemini_files(&root.dir),
        };
        for path in files {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let updated_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            out.push(Candidate {
                provider: root.provider,
                account: root.account.clone(),
                path,
                updated_ms,
                size_bytes: meta.len(),
            });
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.updated_ms));
    out
}

fn dirs_in(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

fn files_in(p: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(&keep))
        .collect()
}

/// `projects/<encoded-cwd>/<id>.jsonl` — top level only; subagent
/// transcripts live in per-session subfolders and are not conversations.
fn claude_files(dir: &Path) -> Vec<PathBuf> {
    dirs_in(&dir.join("projects"))
        .iter()
        .flat_map(|p| files_in(p, |n| n.ends_with(".jsonl")))
        .collect()
}

/// `sessions/YYYY/MM/DD/rollout-*.jsonl[.zst]`.
fn codex_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for y in dirs_in(&dir.join("sessions")) {
        for m in dirs_in(&y) {
            for d in dirs_in(&m) {
                out.extend(files_in(&d, |n| {
                    n.starts_with("rollout-") && (n.ends_with(".jsonl") || n.ends_with(".jsonl.zst"))
                }));
            }
        }
    }
    out
}

/// `tmp/<project_hash>/chats/*.jsonl` (+ legacy `*.json`) — top level only;
/// subagent chats are nested under their parent's id.
fn gemini_files(dir: &Path) -> Vec<PathBuf> {
    dirs_in(&dir.join("tmp"))
        .iter()
        .flat_map(|p| files_in(&p.join("chats"), |n| n.ends_with(".jsonl") || n.ends_with(".json")))
        .collect()
}

/// Lines of a plain or zstd-compressed JSONL file.
fn lines(path: &Path) -> Result<Box<dyn Iterator<Item = String>>> {
    let file = std::fs::File::open(path)?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zst") {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    // A read error mid-file ends the iteration rather than spinning on it.
    Ok(Box::new(BufReader::new(reader).lines().map_while(std::result::Result::ok)))
}

fn json_lines(path: &Path) -> Result<Box<dyn Iterator<Item = Value>>> {
    Ok(Box::new(lines(path)?.filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())))
}

fn is_legacy_gemini(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "json")
}

fn file_stem_id(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    name.trim_end_matches(".zst")
        .trim_end_matches(".jsonl")
        .trim_end_matches(".json")
        .to_string()
}

/// Read the head of `c`'s file for its id, cwd and title.
fn peek(c: Candidate) -> ExternalSession {
    let mut s = ExternalSession {
        provider: c.provider,
        account: c.account,
        id: file_stem_id(&c.path),
        path: c.path,
        cwd: None,
        title: String::new(),
        updated_ms: c.updated_ms,
        size_bytes: c.size_bytes,
        snippet: None,
    };
    let mut summary = None;
    let values: Box<dyn Iterator<Item = Value>> = if s.provider == Provider::Gemini && is_legacy_gemini(&s.path) {
        Box::new(legacy_gemini_records(&s.path).into_iter())
    } else {
        match json_lines(&s.path) {
            Ok(it) => it,
            Err(_) => return s,
        }
    };
    for v in values.take(PEEK_LINES) {
        match s.provider {
            Provider::Claude => {
                if s.cwd.is_none() {
                    s.cwd = v["cwd"].as_str().map(str::to_string);
                }
                if let Some(id) = v["sessionId"].as_str() {
                    s.id = id.to_string();
                }
                if v["type"] == "summary" && summary.is_none() {
                    summary = v["summary"].as_str().map(str::to_string);
                }
            }
            Provider::Codex => {
                if v["type"] == "session_meta" {
                    let p = &v["payload"];
                    if let Some(id) = p["id"].as_str() {
                        s.id = id.to_string();
                    }
                    s.cwd = p["cwd"].as_str().map(str::to_string);
                }
            }
            Provider::Gemini => {
                if v.get("projectHash").is_some() {
                    if let Some(id) = v["sessionId"].as_str() {
                        s.id = id.to_string();
                    }
                    if let Some(sum) = v["summary"].as_str() {
                        summary = Some(sum.to_string());
                    }
                    if s.cwd.is_none() {
                        s.cwd = v["directories"][0].as_str().map(str::to_string);
                    }
                }
            }
        }
        if s.title.is_empty() {
            if let Some(t) = turns_of(s.provider, &v).into_iter().find(|t| t.role == TurnRole::User) {
                s.title = one_line(&t.text, TITLE_MAX_CHARS);
            }
        }
        if !s.title.is_empty() && s.cwd.is_some() && s.provider != Provider::Claude {
            break;
        }
    }
    if let Some(sum) = summary {
        s.title = one_line(&sum, TITLE_MAX_CHARS);
    }
    if s.title.is_empty() {
        s.title = "(ไม่มีข้อความ)".into();
    }
    s
}

/// Sessions on disk under `roots`, newest first. Only the newest `limit`
/// files are opened.
pub fn discover(roots: &[Root], limit: usize) -> Vec<ExternalSession> {
    candidates(roots).into_iter().take(limit).map(peek).collect()
}

/// Sessions whose title, cwd or id contains `query` (case-insensitive), or
/// whose conversation does. Newest first, at most `limit`.
pub fn search(roots: &[Root], query: &str, limit: usize) -> Vec<ExternalSession> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return discover(roots, limit);
    }
    let mut out = Vec::new();
    for c in candidates(roots).into_iter().take(SEARCH_MAX_FILES) {
        if out.len() >= limit {
            break;
        }
        let too_big = c.size_bytes > SEARCH_MAX_BYTES;
        let mut s = peek(c);
        let meta_hit = s.title.to_lowercase().contains(&q)
            || s.id.to_lowercase().contains(&q)
            || s.cwd.as_deref().is_some_and(|d| d.to_lowercase().contains(&q));
        if meta_hit {
            out.push(s);
            continue;
        }
        if too_big {
            continue;
        }
        if let Ok(turns) = read_turns(&s) {
            if let Some(t) = turns.iter().find(|t| t.text.to_lowercase().contains(&q)) {
                s.snippet = Some(snippet_around(&t.text, &q));
                out.push(s);
            }
        }
    }
    out
}

/// Shortest id fragment accepted as a prefix/substring match. Anything
/// shorter is too likely to name the wrong conversation.
const MIN_PARTIAL_ID: usize = 8;

/// Find one session by provider and its path, full id, or (8+ characters of)
/// the id. A fragment that matches more than one session finds none.
pub fn find(roots: &[Root], provider: Provider, id_or_path: &str) -> Option<ExternalSession> {
    let p = Path::new(id_or_path);
    let partial_ok = id_or_path.chars().count() >= MIN_PARTIAL_ID;
    let matches: Vec<ExternalSession> = candidates(roots)
        .into_iter()
        .filter(|c| c.provider == provider)
        // Claude and Codex put the full id in the file name; Gemini names
        // files by a short id, so its files are opened to read the real one.
        .filter(|c| c.path == p || file_stem_id(&c.path).contains(id_or_path) || c.provider == Provider::Gemini)
        .map(peek)
        .filter(|s| s.path == p || s.id == id_or_path || (partial_ok && (s.id.contains(id_or_path) || file_stem_id(&s.path).contains(id_or_path))))
        .collect();
    if let Some(exact) = matches.iter().find(|s| s.path == p || s.id == id_or_path) {
        return Some(exact.clone());
    }
    match matches.len() {
        1 => matches.into_iter().next(),
        _ => None,
    }
}

// ── reading ──────────────────────────────────────────────────────────────

/// The whole conversation, in order, with adjacent text from the same side
/// merged.
pub fn read_turns(s: &ExternalSession) -> Result<Vec<ExternalTurn>> {
    Ok(merge(raw_turns(s)?))
}

/// One entry per content block, empty ones dropped. Import counts these:
/// merged turns are not stable as a file grows (a new user line merges into
/// the previous one), so a count of them cannot say what is new.
fn raw_turns(s: &ExternalSession) -> Result<Vec<ExternalTurn>> {
    let raw: Vec<ExternalTurn> = match s.provider {
        Provider::Gemini => gemini_messages(&s.path)?
            .iter()
            .flat_map(|v| turns_of(Provider::Gemini, v))
            .collect(),
        p => json_lines(&s.path)?.flat_map(|v| turns_of(p, &v)).collect(),
    };
    Ok(raw.into_iter().filter(|t| !t.text.trim().is_empty()).collect())
}

fn merge(raw: Vec<ExternalTurn>) -> Vec<ExternalTurn> {
    let mut out: Vec<ExternalTurn> = Vec::new();
    for t in raw {
        match out.last_mut() {
            Some(last) if last.role == t.role && matches!(t.role, TurnRole::User | TurnRole::Agent) => {
                last.text.push_str("\n\n");
                last.text.push_str(&t.text);
            }
            _ => out.push(t),
        }
    }
    for t in &mut out {
        let cap = match t.role {
            TurnRole::ToolCall | TurnRole::ToolResult => TOOL_MAX_CHARS,
            _ => TEXT_MAX_CHARS,
        };
        t.text = clip(&t.text, cap);
    }
    out
}

/// Turns carried by one parsed line/record.
fn turns_of(provider: Provider, v: &Value) -> Vec<ExternalTurn> {
    match provider {
        Provider::Claude => claude_turns(v),
        Provider::Codex => codex_turns(v),
        Provider::Gemini => gemini_turns(v),
    }
}

fn turn(role: TurnRole, text: impl Into<String>) -> ExternalTurn {
    ExternalTurn { role, text: text.into() }
}

fn claude_turns(v: &Value) -> Vec<ExternalTurn> {
    // Sidechains are subagent threads; meta lines are the CLI talking to itself.
    if v["isSidechain"] == true || v["isMeta"] == true {
        return Vec::new();
    }
    let side = match v["type"].as_str() {
        Some("user") => TurnRole::User,
        Some("assistant") => TurnRole::Agent,
        _ => return Vec::new(),
    };
    let content = &v["message"]["content"];
    if let Some(text) = content.as_str() {
        return if side == TurnRole::User && is_wrapper_noise(text) { Vec::new() } else { vec![turn(side, text)] };
    }
    let mut out = Vec::new();
    for block in content.as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => {
                let text = block["text"].as_str().unwrap_or_default();
                if !(side == TurnRole::User && is_wrapper_noise(text)) {
                    out.push(turn(side, text));
                }
            }
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or("tool");
                out.push(turn(TurnRole::ToolCall, format!("{name}: {}", compact(&block["input"]))));
            }
            Some("tool_result") => out.push(turn(TurnRole::ToolResult, parts_text(&block["content"]))),
            _ => {}
        }
    }
    out
}

fn codex_turns(v: &Value) -> Vec<ExternalTurn> {
    // `event_msg` repeats what `response_item` says for the UI; reading both
    // would record every message twice.
    if v["type"] != "response_item" {
        return Vec::new();
    }
    let p = &v["payload"];
    match p["type"].as_str() {
        Some("message") => {
            let side = match p["role"].as_str() {
                Some("user") => TurnRole::User,
                Some("assistant") => TurnRole::Agent,
                _ => return Vec::new(), // developer/system instructions
            };
            p["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["text"].as_str())
                .filter(|t| !(side == TurnRole::User && is_wrapper_noise(t)))
                .map(|t| turn(side, t))
                .collect()
        }
        Some("function_call") | Some("custom_tool_call") => {
            let name = p["name"].as_str().unwrap_or("tool");
            let args = p.get("arguments").or_else(|| p.get("input")).map(compact).unwrap_or_default();
            vec![turn(TurnRole::ToolCall, format!("{name}: {args}"))]
        }
        Some("local_shell_call") => {
            vec![turn(TurnRole::ToolCall, format!("shell: {}", compact(&p["action"]["command"])))]
        }
        Some("function_call_output") | Some("custom_tool_call_output") => {
            let o = &p["output"];
            let text = o.as_str().map(str::to_string).unwrap_or_else(|| parts_text(&o["content"]));
            vec![turn(TurnRole::ToolResult, text)]
        }
        _ => Vec::new(), // reasoning, web_search_call, …
    }
}

fn gemini_turns(v: &Value) -> Vec<ExternalTurn> {
    match v["type"].as_str() {
        Some("user") => {
            let text = parts_text(&v["content"]);
            if is_wrapper_noise(&text) { Vec::new() } else { vec![turn(TurnRole::User, text)] }
        }
        Some("gemini") => {
            let mut out = vec![turn(TurnRole::Agent, parts_text(&v["content"]))];
            for call in v["toolCalls"].as_array().into_iter().flatten() {
                let name = call["name"].as_str().unwrap_or("tool");
                out.push(turn(TurnRole::ToolCall, format!("{name}: {}", compact(&call["args"]))));
                if !call["result"].is_null() {
                    out.push(turn(TurnRole::ToolResult, parts_text(&call["result"])));
                }
            }
            out
        }
        _ => Vec::new(), // info / error / warning are CLI chrome
    }
}

/// The message records of a Gemini chat after replaying `$rewindTo`, `$set`
/// and `$patch` the way the CLI does when it loads one.
fn gemini_messages(path: &Path) -> Result<Vec<Value>> {
    let records: Vec<Value> = if is_legacy_gemini(path) {
        legacy_gemini_records(path)
    } else {
        json_lines(path)?.collect()
    };
    let mut msgs: Vec<Value> = Vec::new();
    for r in records {
        if let Some(id) = r["$rewindTo"].as_str() {
            if let Some(i) = msgs.iter().position(|m| m["id"] == id) {
                msgs.truncate(i);
            }
        } else if let Some(set) = r.get("$set") {
            if let Some(all) = set["messages"].as_array() {
                msgs = all.clone();
            }
        } else if let Some(patch) = r.get("$patch") {
            let mut apply = |p: &Value| {
                if let Some(m) = msgs.iter_mut().find(|m| m["id"] == p["id"]) {
                    if let Some(c) = p.get("content") {
                        m["content"] = c.clone();
                    }
                }
            };
            apply(patch);
            for u in patch["updates"].as_array().into_iter().flatten() {
                apply(u);
            }
            if let Some(ids) = patch["removeIds"].as_array() {
                msgs.retain(|m| !ids.contains(&m["id"]));
            }
        } else if r.get("type").is_some() && r.get("id").is_some() {
            msgs.push(r);
        }
    }
    Ok(msgs)
}

/// A legacy `.json` chat: one `ConversationRecord`, flattened to the same
/// metadata-then-messages stream the JSONL form has.
fn legacy_gemini_records(path: &Path) -> Vec<Value> {
    let ok_size = std::fs::metadata(path).is_ok_and(|m| m.len() <= LEGACY_JSON_MAX_BYTES);
    let Some(mut v) = ok_size
        .then(|| std::fs::read(path).ok())
        .flatten()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    else {
        return Vec::new();
    };
    let messages = v.as_object_mut().and_then(|o| o.remove("messages"));
    let mut out = vec![v];
    if let Some(Value::Array(m)) = messages {
        out.extend(m);
    }
    out
}

/// Text out of a string, a list of parts/blocks, or a single part.
fn parts_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(parts_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(o) => {
            if let Some(t) = o.get("text").and_then(Value::as_str) {
                t.to_string()
            } else if let Some(r) = o.get("functionResponse") {
                compact(&r["response"])
            } else if let Some(c) = o.get("content") {
                parts_text(c)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Wrappers the CLIs inject as "user" text: environment blocks, AGENTS.md,
/// slash-command echoes. Not something the person typed.
fn is_wrapper_noise(text: &str) -> bool {
    let t = text.trim_start();
    const TAGS: &[&str] = &[
        "<environment_context",
        "<user_instructions",
        "<permissions",
        "<INSTRUCTIONS",
        "<system-reminder",
        "<command-name",
        "<command-message",
        "<local-command",
        "<turn_aborted",
        "# AGENTS.md instructions",
        "Caveat: The messages below were generated",
    ];
    TAGS.iter().any(|p| t.starts_with(p))
}

fn compact(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn clip(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}… (ตัดออก {} ตัวอักษร)", &s[..i], s[i..].chars().count()),
        None => s.to_string(),
    }
}

fn one_line(s: &str, max_chars: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

fn snippet_around(text: &str, q_lower: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_lowercase();
    // Byte offsets of `lower` and `flat` diverge for non-ASCII case folds, so
    // map through a char index instead of slicing `flat` with `lower`'s bytes.
    let char_at = lower.find(q_lower).map(|b| lower[..b].chars().count()).unwrap_or(0);
    let start = char_at.saturating_sub(40);
    let piece: String = flat.chars().skip(start).take(120).collect();
    if start > 0 { format!("…{piece}") } else { piece }
}

// ── import ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub session_id: i64,
    /// Events written by this call.
    pub events: usize,
    /// True when the session had been imported before.
    pub reused: bool,
    /// Turns the source gained since the last import that were *not*
    /// appended because the conversation already continued in Chorus.
    pub not_appended: usize,
}

/// The marker brief we leave at the end of an import, found by its source.
struct Marker {
    seq: i64,
    sess: i64,
    turns: usize,
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn find_marker(c: &Connection, source: &str) -> Result<Option<Marker>> {
    let needle = serde_json::to_string(source)?; // quoted, as it appears in the body
    let pattern = format!("%\"source\":{}%", like_escape(&needle));
    let row = c
        .query_row(
            "SELECT seq, sess, body FROM event WHERE kind = ?1 AND body LIKE ?2 ESCAPE '\\'
             ORDER BY seq DESC LIMIT 1",
            rusqlite::params![EventKind::Handoff.as_i64(), pattern],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
        )
        .optional()?;
    Ok(row.map(|(seq, sess, body)| {
        let turns = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["turns"].as_u64())
            .unwrap_or(0) as usize;
        Marker { seq, sess, turns }
    }))
}

/// A handoff brief from the source tool to Chorus. It doubles as the import
/// marker (`source`, `turns`) so no new table is needed, and `task_state`
/// reads its goal like any other brief.
fn marker_body(s: &ExternalSession, goal: &str, turns: usize) -> Result<String> {
    let mut goal = one_line(goal, 240);
    loop {
        let body = serde_json::json!({
            "goal": goal,
            "done": [],
            "pending": [],
            "constraints": [],
            "files": [],
            "diff": "",
            "handoff": { "from": s.agent_name(), "to": "chorus", "reason": "manual", "at": 0 },
            "source": s.source_key(),
            "source_path": s.path.to_string_lossy(),
            "turns": turns,
        })
        .to_string();
        // The marker must stay inline: a body moved to the blob store is
        // invisible to the `LIKE` lookup in `find_marker`.
        if body.len() < INLINE_MAX_BYTES || goal.is_empty() {
            return Ok(body);
        }
        goal = one_line(&goal, goal.chars().count() / 2);
    }
}

fn append_turns(c: &Connection, sess: i64, user: i64, agent: i64, turns: &[ExternalTurn]) -> Result<usize> {
    for t in turns {
        let (kind, who) = match t.role {
            TurnRole::User => (EventKind::Msg, user),
            TurnRole::Agent => (EventKind::Msg, agent),
            TurnRole::ToolCall => (EventKind::ToolCall, agent),
            TurnRole::ToolResult => (EventKind::ToolResult, agent),
        };
        ledger::append(c, &NewEvent::new(sess, kind, who, t.text.clone()))?;
    }
    Ok(turns.len())
}

/// Import `s` into the ledger as a session rooted at `cwd`, or bring an
/// earlier import up to date. Importing the same source twice never makes a
/// second session: new turns are appended when the Chorus side has not moved
/// on, and reported (not interleaved) when it has.
pub fn import(c: &Connection, s: &ExternalSession, cwd: &str) -> Result<ImportReport> {
    let turns = raw_turns(s)?;
    if turns.is_empty() {
        return Err(ChorusError::Invalid(format!("{} session {} has no messages", s.provider.label(), s.id)));
    }
    let goal = turns.iter().find(|t| t.role == TurnRole::User).map(|t| t.text.as_str()).unwrap_or_default();
    let tx = c.unchecked_transaction()?;
    let user = actor::intern(&tx, "user", "human")?;
    let agent = actor::intern(&tx, &s.agent_name(), s.provider.vendor())?;
    let source = s.source_key();

    let report = if let Some(m) = find_marker(&tx, &source)? {
        let moved_on: i64 = tx.query_row(
            "SELECT COUNT(*) FROM event WHERE sess = ?1 AND seq > ?2",
            [m.sess, m.seq],
            |r| r.get(0),
        )?;
        let fresh = turns.len().saturating_sub(m.turns);
        if fresh == 0 {
            ImportReport { session_id: m.sess, events: 0, reused: true, not_appended: 0 }
        } else if moved_on > 0 {
            ImportReport { session_id: m.sess, events: 0, reused: true, not_appended: fresh }
        } else {
            let n = append_turns(&tx, m.sess, user, agent, &merge(turns[m.turns..].to_vec()))?;
            ledger::append(&tx, &NewEvent::new(m.sess, EventKind::Handoff, agent, marker_body(s, goal, turns.len())?))?;
            ImportReport { session_id: m.sess, events: n + 1, reused: true, not_appended: 0 }
        }
    } else {
        let title = format!("{} · {}", s.provider.label(), s.title);
        let sess = ledger::new_session(&tx, &title, cwd)?;
        let n = append_turns(&tx, sess, user, agent, &merge(turns.clone()))?;
        ledger::append(&tx, &NewEvent::new(sess, EventKind::Handoff, agent, marker_body(s, goal, turns.len())?))?;
        ImportReport { session_id: sess, events: n + 1, reused: false, not_appended: 0 }
    };
    tx.commit()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_noise_is_not_a_user_message() {
        assert!(is_wrapper_noise("<environment_context>\n<cwd>/x</cwd>"));
        assert!(is_wrapper_noise("  <command-name>/clear</command-name>"));
        assert!(!is_wrapper_noise("แก้บั๊ก <div> ใน header"));
    }

    #[test]
    fn clip_counts_thai_by_character_not_byte() {
        let s = "ก".repeat(10);
        assert_eq!(clip(&s, 10), s);
        assert!(clip(&s, 4).starts_with("กกกก…"));
    }

    #[test]
    fn snippet_handles_non_ascii_case_folding() {
        // `İ` lowercases to two chars; slicing `flat` by `lower`'s byte
        // offsets would panic or cut mid-character.
        let s = snippet_around("İİİ ทดสอบ needle ท้าย", "needle");
        assert!(s.contains("needle"));
    }
}
