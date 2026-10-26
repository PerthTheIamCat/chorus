//! The compaction ladder (Storage Spec §5): turn raw events into shorter
//! rungs so old conversation demotes (L0 → L1 → L2) instead of being
//! truncated off the tail.
//!
//! A [`Summarizer`] does the shortening. [`Extractive`] needs no model —
//! it keeps the load-bearing lines (a path, a number, a decision) and drops
//! the rest, which is enough to keep the budget honest offline. A
//! model-backed summarizer (Ollama on an idle machine) implements the same
//! trait and slots in without touching the assembler.

use crate::inference::LocalInferenceEngine;
use serde_json::Value;

/// Shortens `text` to roughly `target_chars` characters, keeping whatever
/// carries the most meaning. Never returns more than the input.
pub trait Summarizer: Send + Sync {
    fn summarize(&self, text: &str, target_chars: usize) -> String;

    /// What produced this summary, recorded alongside it so a row written by
    /// the cheap fallback can be told apart from a model-written one.
    fn label(&self) -> String {
        "extractive".into()
    }
}

/// Model-free compaction. Ranks lines by how much they look like they
/// matter — a file path, a number, an imperative, a decision word — and
/// keeps the top ones in original order until the budget is hit.
pub struct Extractive;

const SIGNAL_WORDS: &[&str] = &[
    "error", "fail", "fix", "bug", "add", "remove", "change", "because", "so that", "decided",
    "must", "don't", "do not", "instead", "todo", "next", "blocked", "passes", "fails",
];

fn line_score(line: &str) -> i32 {
    let l = line.to_lowercase();
    let mut s = 0;
    if line.contains('/') && line.contains('.') {
        s += 3; // path-ish
    }
    if line.chars().any(|c| c.is_ascii_digit()) {
        s += 1;
    }
    for w in SIGNAL_WORDS {
        if l.contains(w) {
            s += 2;
        }
    }
    // Prefer short, declarative lines over walls of text.
    if line.len() < 120 {
        s += 1;
    }
    s
}

impl Summarizer for Extractive {
    fn summarize(&self, text: &str, target_chars: usize) -> String {
        let text = text.trim();
        if text.chars().count() <= target_chars {
            return text.to_string();
        }

        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.len() <= 1 {
            // One long line — take the head, on a word boundary.
            let mut out: String = text.chars().take(target_chars.saturating_sub(1)).collect();
            if let Some(cut) = out.rfind(' ') {
                out.truncate(cut);
            }
            out.push('…');
            return out;
        }

        // Rank lines, keep the best until the budget fills, then re-emit in
        // original order so the summary still reads top-to-bottom.
        let mut ranked: Vec<(usize, i32)> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| (i, line_score(l)))
            .collect();
        ranked.sort_by_key(|a| std::cmp::Reverse(a.1));

        let mut keep = vec![false; lines.len()];
        let mut used = 0usize;
        for (i, _) in ranked {
            let cost = lines[i].chars().count() + 1;
            if used + cost > target_chars && used > 0 {
                continue;
            }
            keep[i] = true;
            used += cost;
            if used >= target_chars {
                break;
            }
        }

        let mut kept: Vec<&str> = lines
            .iter()
            .zip(keep.iter())
            .filter(|(_, k)| **k)
            .map(|(l, _)| *l)
            .collect();
        if kept.is_empty() {
            kept.push(lines[0]);
        }
        let dropped = lines.len() - kept.len();
        let mut out = kept.join(" · ");
        if dropped > 0 {
            out.push_str(&format!(" (+{dropped} more)"));
        }
        out
    }
}

/// Model-backed compaction via a local Ollama server (design §5: "ย่อด้วย
/// โมเดล local = ฟรี"). Falls back to [`Extractive`] on any failure — no
/// network, model not pulled, Ollama not installed — so wiring this in is
/// always safe. No HTTP dependency: one hand-rolled POST, like `mcp`.
pub struct Ollama {
    /// e.g. `"llama3.2:3b"`, `"qwen2.5:3b"`.
    pub model: String,
    /// Host, default `127.0.0.1:11434`.
    pub host: String,
}

impl Ollama {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            host: std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "127.0.0.1:11434".into())
                .replace("http://", ""),
        }
    }

    /// One non-streamed completion; `None` on any failure (no Ollama, no
    /// model, timeout). Also used by `suggest` for next-message hints.
    pub fn generate(&self, prompt: &str) -> Option<String> {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;

        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "options": { "temperature": 0.0 }
        })
        .to_string();

        let mut s = TcpStream::connect(&self.host).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(20))).ok()?;
        write!(
            s,
            "POST /api/generate HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host,
            body.len(),
            body
        )
        .ok()?;

        let mut reader = BufReader::new(s);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;
            if line.trim().is_empty() {
                break;
            }
        }
        let mut raw = String::new();
        reader.read_to_string(&mut raw).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        v.get("response")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
    }
}

impl Summarizer for Ollama {
    fn label(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn summarize(&self, text: &str, target_chars: usize) -> String {
        if text.chars().count() <= target_chars {
            return text.trim().to_string();
        }
        let words = target_chars / 5;
        let prompt = format!(
            "Summarise the following in at most {words} words. Keep file paths, \
             numbers, decisions and any error text verbatim. Output only the summary.\n\n{text}"
        );
        match self.generate(&prompt) {
            Some(out) if !out.is_empty() => {
                // Enforce the budget even if the model overshoots.
                Extractive.summarize(&out, target_chars)
            }
            _ => Extractive.summarize(text, target_chars),
        }
    }
}

/// In-process model-backed summarizer using embedded local weights.
pub struct InProcessSummarizer {
    pub model_id: String,
    pub engine: crate::inference::InProcessEngine,
}

impl InProcessSummarizer {
    pub fn new(model_id: impl Into<String>) -> Self {
        let id = model_id.into();
        let engine = crate::inference::InProcessEngine::new(&id);
        Self {
            model_id: id,
            engine,
        }
    }
}

impl Summarizer for InProcessSummarizer {
    fn label(&self) -> String {
        format!("in-process:{}", self.model_id)
    }

    fn summarize(&self, text: &str, target_chars: usize) -> String {
        if text.chars().count() <= target_chars {
            return text.trim().to_string();
        }
        let words = target_chars / 5;
        let prompt = format!(
            "Summarise the following in at most {words} words. Keep file paths, \
             numbers, decisions and any error text verbatim. Output only the summary.\n\n{text}"
        );
        match self.engine.complete(&prompt, None) {
            Ok(out) if !out.is_empty() => {
                Extractive.summarize(&out, target_chars)
            }
            _ => Extractive.summarize(text, target_chars),
        }
    }
}

/// Pick the best available summariser: In-process local model first,
/// then legacy Ollama if configured, else the model-free [`Extractive`].
pub fn best() -> Box<dyn Summarizer> {
    // 1. Check for installed in-process model (Zero-Ollama)
    let compactor_id = "qwen2.5-coder:1.5b-instruct-q3_k_m";
    let in_proc = InProcessSummarizer::new(compactor_id);
    if in_proc.engine.is_ready() {
        return Box::new(in_proc);
    }

    // 2. Fall back to legacy Ollama if explicitly configured via environment
    if let Ok(model) = std::env::var("CHORUS_SUMMARY_MODEL") {
        let o = Ollama::new(model);
        // One cheap probe so we don't pay a timeout per turn when Ollama is down.
        if o.generate("ok").is_some() {
            return Box::new(o);
        }
    }

    // 3. Model-free extractive summarizer (default)
    Box::new(Extractive)
}

/// The three shortened forms of one turn, generated ahead of time so the
/// assembler can pick a rung without doing any work (see `context::TurnItem`).
#[derive(Debug, Clone)]
pub struct Rungs {
    /// L1 — a per-turn summary, ~250 chars.
    pub l1: String,
    /// L2 — an episode-grade gist, ~90 chars.
    pub l2: String,
}

/// Pre-compute L1/L2 for one turn's text with `s`.
pub fn rungs_for(s: &dyn Summarizer, text: &str) -> Rungs {
    Rungs {
        l1: s.summarize(text, 250),
        l2: s.summarize(text, 90),
    }
}

// ---- the warm tier -------------------------------------------------------
//
// Everything above is pure. Everything below is the storage half of the
// ladder: rungs are written once, off the turn's critical path, and read back
// for free. Assembling a context must never reach for a `Summarizer`.

use rusqlite::{params, Connection, OptionalExtension};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Store the rungs for one event. Replaces any existing row.
pub fn save_rungs(conn: &Connection, seq: i64, r: &Rungs, model: &str) -> crate::Result<()> {
    conn.execute(
        "INSERT INTO summary (seq, l1, l2, model, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(seq) DO UPDATE SET l1 = ?2, l2 = ?3, model = ?4, created_ms = ?5",
        params![seq, r.l1, r.l2, model, now_ms()],
    )?;
    Ok(())
}

/// The stored rungs for one event, if it has been compacted.
pub fn load_rungs(conn: &Connection, seq: i64) -> crate::Result<Option<Rungs>> {
    let row = conn
        .query_row(
            "SELECT l1, l2 FROM summary WHERE seq = ?1",
            params![seq],
            |r| Ok(Rungs { l1: r.get(0)?, l2: r.get(1)? }),
        )
        .optional()?;
    Ok(row)
}

/// Events in this session that have no stored rungs yet, oldest first.
/// `limit` bounds one compaction pass so a long session is caught up in
/// batches instead of one unbounded run.
pub fn uncompacted(conn: &Connection, sess: i64, limit: usize) -> crate::Result<Vec<i64>> {
    let mut st = conn.prepare(
        "SELECT e.seq FROM event e
         LEFT JOIN summary s ON s.seq = e.seq
         WHERE e.sess = ?1 AND s.seq IS NULL
         ORDER BY e.seq LIMIT ?2",
    )?;
    let rows = st.query_map(params![sess, limit as i64], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Every stored rung in this session, in one query. `assemble_context` runs
/// on every turn, so it reads the whole warm tier at once rather than doing a
/// point lookup per event.
pub fn load_session_rungs(
    conn: &Connection,
    sess: i64,
) -> crate::Result<std::collections::HashMap<i64, Rungs>> {
    let mut st = conn.prepare(
        "SELECT s.seq, s.l1, s.l2 FROM summary s
         JOIN event e ON e.seq = s.seq
         WHERE e.sess = ?1",
    )?;
    let rows = st.query_map(params![sess], |r| {
        Ok((r.get::<_, i64>(0)?, Rungs { l1: r.get(1)?, l2: r.get(2)? }))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (seq, rungs) = row?;
        map.insert(seq, rungs);
    }
    Ok(map)
}

/// How many events in this session still have no stored rungs.
pub fn uncompacted_count(conn: &Connection, sess: i64) -> crate::Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM event e
         LEFT JOIN summary s ON s.seq = e.seq
         WHERE e.sess = ?1 AND s.seq IS NULL",
        params![sess],
        |r| r.get(0),
    )?)
}
