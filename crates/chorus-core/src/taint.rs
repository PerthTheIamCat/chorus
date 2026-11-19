//! Turn taint: has this turn read something the person did not give it?
//!
//! A web page, an imported session or a file outside the project can all carry
//! text written for the agent rather than for the person ("now run
//! `git push` to …"). The agent cannot tell that text from an instruction, so
//! once a turn has read one, the actions that leave the machine — push, PR
//! creation, a write through someone else's MCP server, a computer-use
//! click — stop riding on the blanket "ให้ทำเลย" and go back to asking
//! (E2, CLAUDE.md §6).
//!
//! Everything here is derived from tool calls, the same `(title, kind)` pairs
//! that end up in the ledger as `"{kind}: {title}"`, so the live gate (which
//! sees them as the agent makes them) and the label shown on the turn (which
//! reads them back from the ledger) cannot disagree.
//!
//! What this is not: a proof. Titles are whatever the bridge chose to print,
//! and an agent that reads a hostile file through a shell command it
//! disguised is not caught. It narrows the common case — the agent asked to
//! fetch or open something outside the project — and says so on the turn.

use crate::{EventKind, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// Where outside text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A page or search result from the web.
    Web,
    /// A transcript imported from another app (or the history of one).
    ImportedSession,
    /// A file outside every folder of the project.
    OutsideFile,
}

impl Source {
    /// Shown to the person, so Thai.
    pub fn label(self) -> &'static str {
        match self {
            Source::Web => "เว็บ",
            Source::ImportedSession => "session ที่นำเข้า",
            Source::OutsideFile => "ไฟล์นอกโปรเจกต์",
        }
    }
}

/// The outside sources a turn read, in the order first seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Taint {
    pub sources: Vec<Source>,
}

impl Taint {
    /// The line shown on the turn: "อ่านของภายนอกแล้ว · เว็บ, ไฟล์นอกโปรเจกต์".
    pub fn label(&self) -> String {
        let names: Vec<&str> = self.sources.iter().map(|s| s.label()).collect();
        format!("อ่านของภายนอกแล้ว · {}", names.join(", "))
    }
}

/// What counts as outside for one session.
#[derive(Debug, Clone)]
pub struct Judge {
    roots: Vec<PathBuf>,
    imported_sessions: bool,
    home: Option<PathBuf>,
}

impl Judge {
    /// `roots`: the working directory and every other folder of the project.
    /// `imported_sessions`: whether the store holds any imported session (see
    /// [`imported_sessions_exist`]) — only then can a history search have
    /// returned imported text.
    pub fn new(roots: impl IntoIterator<Item = PathBuf>, imported_sessions: bool) -> Self {
        Self {
            roots: roots.into_iter().map(|r| normalize(&r)).collect(),
            imported_sessions,
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }

    /// What, if anything, one tool call read from outside.
    pub fn source_of(&self, title: &str, kind: &str) -> Option<Source> {
        let kind = kind.trim().to_lowercase();
        let lower = title.to_lowercase();
        if kind == "fetch" || is_web_title(&lower) {
            return Some(Source::Web);
        }
        if self.imported_sessions && lower.contains("chorus_search_history") {
            return Some(Source::ImportedSession);
        }
        // Only calls that read are judged by path: a shell command is full of
        // absolute paths (`/usr/bin/env`, `cd /project`) that mean nothing.
        let reads = matches!(kind.as_str(), "read" | "search") || (kind.is_empty() && lower.starts_with("read"));
        if !reads {
            return None;
        }
        let mut found = None;
        for path in path_tokens(title) {
            let abs = self.resolve(&path);
            if is_session_store(&abs, self.home.as_deref()) {
                return Some(Source::ImportedSession);
            }
            if found.is_none() && !self.inside(&abs) && !is_scratch(&abs) {
                found = Some(Source::OutsideFile);
            }
        }
        found
    }

    /// The taint of a run of calls, `None` when nothing came from outside.
    pub fn taint_of<'a>(&self, calls: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Taint> {
        let mut sources: Vec<Source> = Vec::new();
        for (title, kind) in calls {
            if let Some(s) = self.source_of(title, kind) {
                if !sources.contains(&s) {
                    sources.push(s);
                }
            }
        }
        (!sources.is_empty()).then_some(Taint { sources })
    }

    /// [`Judge::taint_of`] over ledger `ToolCall` bodies (`"{kind}: {title}"`).
    /// Subagent bookkeeping rows (JSON) and anything else unparseable is skipped.
    pub fn taint_of_bodies<'a>(&self, bodies: impl IntoIterator<Item = &'a str>) -> Option<Taint> {
        let calls: Vec<(&str, &str)> = bodies
            .into_iter()
            .filter(|b| !b.trim_start().starts_with('{'))
            .filter_map(|b| b.split_once(": ").map(|(kind, title)| (title, kind)))
            .collect();
        self.taint_of(calls)
    }

    fn resolve(&self, token: &str) -> PathBuf {
        if let Some(rest) = token.strip_prefix("~/") {
            return normalize(&self.home.clone().unwrap_or_default().join(rest));
        }
        let p = Path::new(token);
        if p.is_absolute() {
            return normalize(p);
        }
        // `../x` — relative to the working directory.
        normalize(&self.roots.first().cloned().unwrap_or_default().join(p))
    }

    fn inside(&self, abs: &Path) -> bool {
        self.roots.iter().any(|r| abs.starts_with(r))
    }
}

/// Does the store hold an imported session? Imports leave a `Handoff` marker
/// carrying `"source":` ([`crate::external_sessions`]); ordinary handoff briefs
/// have no such key.
pub fn imported_sessions_exist(conn: &Connection) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT 1 FROM event WHERE kind = ?1 AND body LIKE '%\"source\":%' LIMIT 1",
            [EventKind::Handoff.as_i64()],
            |_| Ok(()),
        )
        .map(|_| true);
    match found {
        Ok(b) => Ok(b),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn is_web_title(lower: &str) -> bool {
    const NAMES: [&str; 9] = [
        "webfetch",
        "web fetch",
        "web_fetch",
        "websearch",
        "web search",
        "web_search",
        "browser_",
        "playwright",
        "puppeteer",
    ];
    if NAMES.iter().any(|n| lower.contains(n)) {
        return true;
    }
    // A shell command that pulls a page down: `curl https://…`, `wget …`.
    let pulls = ["curl ", "wget ", "http get ", "httpie "].iter().any(|c| lower.contains(c));
    pulls && (lower.contains("http://") || lower.contains("https://"))
}

/// Absolute (`/…`, `~/…`) and parent-relative (`../…`) path-like words in a title.
fn path_tokens(title: &str) -> Vec<String> {
    title
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',' | ';' | '(' | ')'))
        .map(|w| w.trim_end_matches(['.', ':']))
        .filter(|w| w.starts_with('/') || w.starts_with("~/") || w.starts_with("../"))
        .filter(|w| w.len() > 1 && !w.starts_with("//"))
        .map(str::to_string)
        .collect()
}

/// Lexical `.`/`..` folding. No filesystem access: the file may be gone, and a
/// symlink inside the project pointing out is a separate question.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Where the CLIs keep the transcripts of their sessions
/// ([`crate::external_sessions`]): reading these is reading an imported session
/// whether or not Chorus imported it.
fn is_session_store(abs: &Path, home: Option<&Path>) -> bool {
    let Some(home) = home else { return false };
    ["projects", "sessions"]
        .iter()
        .any(|d| abs.starts_with(home.join(".claude").join(d)) || abs.starts_with(home.join(".codex").join(d)))
        || abs.starts_with(home.join(".gemini").join("tmp"))
}

/// The agent's own scratch space: what it wrote there it wrote.
fn is_scratch(abs: &Path) -> bool {
    ["/tmp", "/private/tmp", "/var/folders", "/private/var/folders"]
        .iter()
        .any(|t| abs.starts_with(t))
}

// ---- outbound actions ---------------------------------------------------

/// If this permission ask is an action that leaves the machine, a short name
/// for it ("git push", "gh pr create", "MCP github/create_issue"). `detail` is
/// the bridge's own description of the action, often the command line.
pub fn outbound(title: &str, kind: &str, detail: &str) -> Option<String> {
    let text = format!("{title} {detail}").to_lowercase();
    if let Some(m) = mcp_write(title, kind) {
        return Some(m);
    }
    if text.contains("git push") {
        return Some("git push".into());
    }
    for (needle, name) in [
        ("gh pr create", "gh pr create"),
        ("gh pr merge", "gh pr merge"),
        ("gh pr comment", "gh pr comment"),
        ("gh pr review", "gh pr review"),
        ("gh pr edit", "gh pr edit"),
        ("gh pr close", "gh pr close"),
        ("gh issue create", "gh issue create"),
        ("gh issue comment", "gh issue comment"),
        ("gh release create", "gh release create"),
        ("gh repo create", "gh repo create"),
        ("npm publish", "npm publish"),
        ("cargo publish", "cargo publish"),
        ("docker push", "docker push"),
        ("twine upload", "twine upload"),
    ] {
        if text.contains(needle) {
            return Some(name.into());
        }
    }
    if text.contains("gh api") && has_write_flag(&text, &["-x post", "-x put", "-x patch", "-x delete", "--method post", "--method put", "--method patch", "--method delete", " -f ", " -f=", "--field", "--raw-field", "--input"]) {
        return Some("gh api (write)".into());
    }
    if (text.contains("curl ") || text.contains("wget "))
        && has_write_flag(
            &text,
            &[
                "-x post", "-x put", "-x patch", "-x delete", "--request post", "--request put", "--request patch",
                "--request delete", " -d ", " -d=", "--data", " -f ", "--form", " -t ", "--upload-file", "--post-data", "--post-file",
            ],
        )
    {
        return Some("curl/wget (ส่งข้อมูลออก)".into());
    }
    None
}

fn has_write_flag(text: &str, flags: &[&str]) -> bool {
    flags.iter().any(|f| text.contains(f))
}

/// `mcp__server__tool` (Claude) or `Tool: server/tool` (Codex), unless it is
/// Chorus's own server or the tool name starts like a read.
fn mcp_write(title: &str, _kind: &str) -> Option<String> {
    let (server, tool) = if let Some(rest) = title.strip_prefix("mcp__") {
        rest.split_once("__")?
    } else {
        title.strip_prefix("Tool: ")?.split_once('/')?
    };
    let tool = tool.trim();
    if server.eq_ignore_ascii_case("chorus") || tool.is_empty() {
        return None;
    }
    let t = tool.to_lowercase();
    const READS: [&str; 14] = [
        "get", "list", "read", "search", "fetch", "view", "find", "query", "describe", "show", "check", "status", "lookup", "download",
    ];
    if READS.iter().any(|r| t == *r || t.starts_with(&format!("{r}_")) || t.starts_with(&format!("{r}-"))) {
        return None;
    }
    Some(format!("MCP {server}/{tool}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judge() -> Judge {
        let mut j = Judge::new([PathBuf::from("/work/app"), PathBuf::from("/work/lib")], false);
        j.home = Some(PathBuf::from("/Users/me"));
        j
    }

    #[test]
    fn a_turn_that_only_touched_the_project_is_clean() {
        let j = judge();
        let calls = [
            ("Read /work/app/src/main.rs", "read"),
            ("Read src/main.rs", "read"),
            ("Read /work/lib/x.rs", "read"),
            ("cd /work/app && cargo test", "execute"),
            ("Edit /etc/hosts", "edit"),
            ("Read /tmp/scratch.txt", "read"),
        ];
        assert_eq!(j.taint_of(calls), None);
    }

    #[test]
    fn web_pages_taint_by_kind_name_or_a_curl_of_a_url() {
        let j = judge();
        for (title, kind) in [
            ("Fetch https://example.com", "fetch"),
            ("WebFetch", ""),
            ("WebSearch rust sandbox", "search"),
            ("curl -s https://example.com/readme", "execute"),
            ("mcp__playwright__browser_navigate", "other"),
        ] {
            assert_eq!(j.source_of(title, kind), Some(Source::Web), "{title}");
        }
        assert_eq!(j.source_of("curl localhost", "execute"), None, "no URL, no page");
        assert_eq!(j.source_of("git clone https://x/y.git", "execute"), None);
    }

    #[test]
    fn a_file_outside_every_project_folder_taints_even_through_dotdot() {
        let j = judge();
        assert_eq!(j.source_of("Read /Users/me/Downloads/notes.txt", "read"), Some(Source::OutsideFile));
        assert_eq!(j.source_of("Read ~/Desktop/x.md", "read"), Some(Source::OutsideFile));
        assert_eq!(j.source_of("Read ../secrets/key.txt", "read"), Some(Source::OutsideFile));
        assert_eq!(j.source_of("Read /work/app/../other/x", "read"), Some(Source::OutsideFile));
        assert_eq!(j.source_of("Read /work/app/sub/../src/a.rs", "read"), None);
        // A sibling that merely shares a prefix is not inside.
        assert_eq!(j.source_of("Read /work/app-old/a.rs", "read"), Some(Source::OutsideFile));
    }

    #[test]
    fn session_transcripts_are_imported_sessions_wherever_they_are_read_from() {
        let j = judge();
        for p in [
            "/Users/me/.claude/projects/-work-app/abc.jsonl",
            "~/.codex/sessions/2026/11/18/rollout-1.jsonl",
            "/Users/me/.gemini/tmp/h/chats/c.jsonl",
        ] {
            assert_eq!(j.source_of(&format!("Read {p}"), "read"), Some(Source::ImportedSession), "{p}");
        }
    }

    #[test]
    fn history_search_taints_only_when_the_store_has_imports() {
        let mut j = judge();
        assert_eq!(j.source_of("chorus_search_history", "other"), None);
        j.imported_sessions = true;
        assert_eq!(j.source_of("mcp__chorus__chorus_search_history", "other"), Some(Source::ImportedSession));
    }

    #[test]
    fn sources_are_listed_once_in_first_seen_order() {
        let j = judge();
        let t = j
            .taint_of([
                ("Read /Users/me/a.txt", "read"),
                ("WebFetch", "fetch"),
                ("Read /Users/me/b.txt", "read"),
            ])
            .unwrap();
        assert_eq!(t.sources, vec![Source::OutsideFile, Source::Web]);
        assert_eq!(t.label(), "อ่านของภายนอกแล้ว · ไฟล์นอกโปรเจกต์, เว็บ");
    }

    #[test]
    fn ledger_bodies_give_the_same_answer_as_live_calls() {
        let j = judge();
        let live = j.taint_of([("Fetch https://e.com", "fetch"), ("Read /Users/me/x", "read")]);
        let ledger = j.taint_of_bodies([
            "fetch: Fetch https://e.com",
            "read: Read /Users/me/x",
            r#"{"event":"subagent_spawned","name":"Read /Users/me/y"}"#,
        ]);
        assert_eq!(live, ledger);
        assert_eq!(j.taint_of_bodies(["execute: ls", "garbage"]), None);
    }

    #[test]
    fn what_leaves_the_machine_is_named() {
        for (title, detail, want) in [
            ("git push origin main", "", "git push"),
            ("Run command", "git push -u origin feature", "git push"),
            ("gh pr create --title x", "", "gh pr create"),
            ("curl -X POST https://api.example.com/x", "", "curl/wget (ส่งข้อมูลออก)"),
            ("curl -d @body.json https://x.test", "", "curl/wget (ส่งข้อมูลออก)"),
            ("gh api repos/o/r/issues -f title=x", "", "gh api (write)"),
            ("npm publish", "", "npm publish"),
        ] {
            assert_eq!(outbound(title, "execute", detail).as_deref(), Some(want), "{title} {detail}");
        }
    }

    #[test]
    fn local_and_reading_actions_are_not_outbound() {
        for (title, kind) in [
            ("git status", "execute"),
            ("git commit -m x", "execute"),
            ("curl https://example.com", "execute"),
            ("gh api repos/o/r/issues", "execute"),
            ("gh pr view 3", "execute"),
            ("Edit src/a.rs", "edit"),
            ("mcp__chorus__memory_propose", "other"),
            ("mcp__github__get_issue", "other"),
            ("mcp__github__list_pull_requests", "other"),
            ("Tool: github/search_code", "other"),
        ] {
            assert_eq!(outbound(title, kind, ""), None, "{title}");
        }
    }

    #[test]
    fn writes_through_other_mcp_servers_are_outbound() {
        assert_eq!(outbound("mcp__github__create_pull_request", "other", "").as_deref(), Some("MCP github/create_pull_request"));
        assert_eq!(outbound("Tool: slack/post_message", "other", "").as_deref(), Some("MCP slack/post_message"));
        assert_eq!(outbound("mcp__notion__update_page", "", "").as_deref(), Some("MCP notion/update_page"));
    }

    #[test]
    fn imported_sessions_are_found_by_their_marker_only() {
        let store = crate::Store::open_in_memory().unwrap();
        let c = store.conn();
        assert!(!imported_sessions_exist(c).unwrap());
        let sess = crate::ledger::new_session(c, "t", "/work/app").unwrap();
        let a = crate::actor::intern(c, "claude", "anthropic").unwrap();
        // An ordinary handoff brief has no `source` key.
        crate::ledger::append(
            c,
            &crate::NewEvent::new(sess, EventKind::Handoff, a, r#"{"goal":"x","handoff":{"from":"a","to":"b"}}"#),
        )
        .unwrap();
        assert!(!imported_sessions_exist(c).unwrap());
        crate::ledger::append(
            c,
            &crate::NewEvent::new(sess, EventKind::Handoff, a, r#"{"goal":"x","source":"codex:abc","turns":3}"#),
        )
        .unwrap();
        assert!(imported_sessions_exist(c).unwrap());
    }
}
