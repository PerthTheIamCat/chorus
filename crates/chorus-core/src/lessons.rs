//! Lessons drafted from the ledger (memory that learns, L1).
//!
//! Nothing here runs on a turn's path: it is the same off-path pass as
//! compaction (`chorus compact`, the app's idle `compactIfIdle`). It reads what
//! already happened — a command that failed and then passed, hunks the user
//! threw away, a rewind, the user saying "no, not like that", a handoff nobody
//! answered — and drafts one short sentence for each, with the session and
//! seq it came from, into the same approve queue agents use. A person decides;
//! nothing becomes a fact on its own.
//!
//! Rule-based and extractive: the sentence is built from the events
//! themselves, so a lesson never says more than the ledger shows. A local
//! model may reword one (see [`polish`]) only when `CHORUS_SUMMARY_MODEL` is
//! set, and its answer is discarded unless it still fits.

use crate::handoff::HandoffReason;
use crate::memory::{self, NewProposal, ProposalKind};
use crate::types::EventKind;
use crate::{ledger, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;

/// At most this many lessons are queued per pass. A long session can contain
/// dozens of corrections; a review queue that grows by dozens is one nobody
/// reads. The newest win, and the rest are not drafted later (the watermark
/// moves on), which is the point of a cap.
pub const MAX_PER_PASS: usize = 5;

/// No new lessons while this many are already waiting for a person.
pub const MAX_WAITING: usize = 15;

/// A `Decision` event's start when the user threw away hunks (`reject_hunks`).
pub const REJECTED_PREFIX: &str = "ผู้ใช้ปฏิเสธการแก้ไข ";

/// The files a unified diff touches (`+++ b/path` lines), in order, once each.
pub fn patch_files(patch: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line.strip_prefix("+++ ") else { continue };
        let path = rest.split('\t').next().unwrap_or(rest).trim();
        let path = path.strip_prefix("b/").unwrap_or(path);
        if path != "/dev/null" && !path.is_empty() && !out.iter().any(|p| p == path) {
            out.push(path.to_string());
        }
    }
    out
}

/// What the draft came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// A command failed, and the same command passed later.
    FixedAfterFailing,
    /// The user rejected hunks of the agent's change.
    RejectedHunks,
    /// The user rewound to before one of their own requests.
    Rewound,
    /// The user's next message pushes back on the last answer.
    Corrected,
    /// A handoff happened and the incoming agent never answered.
    FailedHandoff,
}

#[derive(Debug, Clone)]
pub struct LessonDraft {
    /// ≤ 140 characters, ready for the queue.
    pub text: String,
    /// `session N · seq M`
    pub evidence: String,
    /// The session had imported or web content before this point: show it with
    /// a warning and never accept it without a person.
    pub suspicious: bool,
    pub signal: Signal,
    /// Newest-first ordering key for the per-pass cap.
    pub seq: i64,
}

/// What [`scan`] found, plus where to resume from.
#[derive(Debug, Clone)]
pub struct Scan {
    pub drafts: Vec<LessonDraft>,
    pub head_seq: i64,
    pub now_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `s` cut to `max` characters, with an ellipsis if anything was lost.
fn fit(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        s
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Final text for a draft: prefix + variable part (cut to fit) + suffix, within
/// the 140-character limit of a fact.
fn compose(prefix: &str, var: &str, suffix: &str) -> String {
    let room = 140usize.saturating_sub(prefix.chars().count() + suffix.chars().count());
    format!("{prefix}{}{suffix}", fit(var, room))
}

const CORRECTION_CUES: &[&str] = &[
    "ไม่ใช่", "ไม่ต้อง", "อย่า", "ผิด", "ไม่ได้ขอ", "ไม่เอา", "ทำใหม่", "ยกเลิก", "หยุด",
    "no,", "no.", "don't", "do not", "stop ", "wrong", "that's not", "that is not", "not what i",
    "undo", "revert", "instead of",
];

fn is_correction(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    // Long messages are new tasks that happen to contain a cue word.
    t.chars().count() <= 200 && CORRECTION_CUES.iter().any(|c| t.contains(c))
}

fn split_title_status(body: &str) -> Option<(&str, &str)> {
    body.rsplit_once(" → ")
}

fn diff_path(body: &str) -> Option<&str> {
    body.lines().next()?.strip_prefix("--- ")
}

fn user_actor(c: &Connection) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT id FROM actor WHERE name = 'user'", [], |r| r.get(0)).optional()?)
}

fn is_import_marker(body: &str) -> bool {
    body.contains("\"source\"") && body.contains("\"to\":\"chorus\"")
}

/// Has the session read the outside world — an imported conversation, or a
/// `fetch` tool call — since the user last spoke? Used to mark what an agent
/// proposes right after reading a web page.
pub fn session_tainted(c: &Connection, sess: i64) -> Result<bool> {
    let Some(user) = user_actor(c)? else { return Ok(false) };
    let events = ledger::history(c, sess, 400)?;
    let mut recent_fetch = false;
    for e in &events {
        let body = ledger::body(c, e)?;
        match e.kind {
            EventKind::Handoff if is_import_marker(&body) => return Ok(true),
            EventKind::Msg if e.actor == user => recent_fetch = false,
            EventKind::ToolCall if body.starts_with("fetch:") => recent_fetch = true,
            _ => {}
        }
    }
    Ok(recent_fetch)
}

struct Failure {
    seq: i64,
    edits: Vec<String>,
}

/// Read the ledger of `sess` (and the chain it forks from) for lessons newer
/// than the session's watermark. Read-only: [`mark_scanned`] moves the
/// watermark once the drafts are queued.
pub fn scan(c: &Connection, sess: i64) -> Result<Scan> {
    let (wm_seq, wm_ms): (i64, i64) = c
        .query_row("SELECT lessons_seq, lessons_ms FROM session WHERE id = ?1", [sess], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .unwrap_or((0, 0));
    let events = ledger::history(c, sess, 5_000)?;
    let head_seq = events.last().map(|e| e.seq).unwrap_or(wm_seq).max(wm_seq);
    let now = now_ms();
    let Some(user) = user_actor(c)? else {
        return Ok(Scan { drafts: Vec::new(), head_seq, now_ms: now });
    };
    let evidence = |seq: i64| format!("session {sess} · seq {seq}");

    let mut drafts: Vec<LessonDraft> = Vec::new();
    let mut tainted = false;
    let mut call_kind: HashMap<String, String> = HashMap::new();
    let mut failed: HashMap<String, Failure> = HashMap::new();
    let mut agent_spoke = false;
    let mut handoff: Option<(i64, String)> = None;

    for e in &events {
        let body = ledger::body(c, e)?;
        let fresh = e.seq > wm_seq;
        let mut push = |d: LessonDraft| {
            if fresh {
                drafts.push(d);
            }
        };
        match e.kind {
            EventKind::ToolCall => {
                if body.starts_with("fetch:") {
                    tainted = true;
                }
                if let Some((kind, title)) = body.split_once(": ") {
                    call_kind.insert(title.to_string(), kind.to_string());
                }
            }
            EventKind::ToolResult => {
                let Some((title, status)) = split_title_status(&body) else { continue };
                let is_cmd = call_kind.get(title).is_some_and(|k| k == "execute");
                if is_cmd && status == "failed" {
                    failed.entry(title.to_string()).or_insert(Failure { seq: e.seq, edits: Vec::new() });
                } else if is_cmd && status == "completed" {
                    if let Some(f) = failed.remove(title) {
                        let cmd = fit(title, 50);
                        let text = if f.edits.is_empty() {
                            compose(&format!("`{cmd}` ล้มก่อน แล้วผ่านในรอบถัดมา — "), "ดูว่าแก้อะไรก่อนรันซ้ำ", "")
                        } else {
                            let files = f.edits.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
                            compose(&format!("`{cmd}` ล้มก่อน แล้วผ่านหลังแก้ "), &files, "")
                        };
                        push(LessonDraft {
                            text,
                            evidence: format!("session {sess} · seq {} → {}", f.seq, e.seq),
                            suspicious: tainted,
                            signal: Signal::FixedAfterFailing,
                            seq: e.seq,
                        });
                    }
                }
            }
            EventKind::Diff => {
                if let Some(p) = diff_path(&body) {
                    for f in failed.values_mut() {
                        if !f.edits.iter().any(|x| x == p) {
                            f.edits.push(p.to_string());
                        }
                    }
                }
            }
            EventKind::Decision => {
                if let Some(files) = body.strip_prefix(REJECTED_PREFIX) {
                    push(LessonDraft {
                        text: compose("ผู้ใช้ปฏิเสธการแก้ ", files.trim(), " — ถามหรือทำให้เล็กลงก่อนแก้ส่วนนี้"),
                        evidence: evidence(e.seq),
                        suspicious: tainted,
                        signal: Signal::RejectedHunks,
                        seq: e.seq,
                    });
                }
            }
            EventKind::Handoff => {
                if is_import_marker(&body) {
                    tainted = true;
                } else if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    let h = &v["handoff"];
                    let reason = serde_json::from_value::<HandoffReason>(h["reason"].clone())
                        .map(HandoffReason::as_phrase)
                        .unwrap_or("handoff");
                    let (from, to) = (h["from"].as_str().unwrap_or("?"), h["to"].as_str().unwrap_or("?"));
                    handoff = Some((e.seq, format!("handoff {from} → {to} ({reason}) ไม่ได้คำตอบ — ตรวจ bridge/โมเดลของ {to} ก่อนส่งไม้")));
                }
            }
            EventKind::Msg if e.actor != user => {
                agent_spoke = true;
                handoff = None;
            }
            EventKind::Msg => {
                // The user speaks. A handoff still unanswered is a failed one.
                if let Some((hseq, text)) = handoff.take() {
                    push(LessonDraft {
                        text: fit(&text, 140),
                        evidence: evidence(hseq),
                        suspicious: tainted,
                        signal: Signal::FailedHandoff,
                        seq: hseq,
                    });
                }
                if agent_spoke && is_correction(&body) {
                    let first = body.split(['\n', '。']).next().unwrap_or(&body);
                    push(LessonDraft {
                        text: compose("ผู้ใช้แก้ทาง: \"", first, "\""),
                        evidence: evidence(e.seq),
                        suspicious: tainted,
                        signal: Signal::Corrected,
                        seq: e.seq,
                    });
                }
            }
            _ => {}
        }
    }

    // Rewinds are forks titled "(ย้อนกลับ)" made since the last pass; the
    // request the user went back before is the first user message after the
    // fork point.
    let mut stmt = c.prepare(
        "SELECT fork_at_seq FROM session
         WHERE forked_from = ?1 AND title LIKE '%(ย้อนกลับ)' AND started_ms > ?2 AND fork_at_seq IS NOT NULL",
    )?;
    let forks: Vec<i64> = stmt.query_map((sess, wm_ms), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for at in forks {
        if let Some(m) = events.iter().find(|e| e.seq > at && e.kind == EventKind::Msg && e.actor == user) {
            let asked = ledger::body(c, m)?;
            drafts.push(LessonDraft {
                text: compose("ผู้ใช้ย้อนกลับหลังคำขอ \"", &asked, "\" — ผลที่ได้ไม่ตรงที่ต้องการ"),
                evidence: evidence(m.seq),
                suspicious: tainted,
                signal: Signal::Rewound,
                seq: m.seq,
            });
        }
    }

    Ok(Scan { drafts, head_seq, now_ms: now })
}

/// Remember how far this session has been read, so the next pass drafts only
/// what is new (and a lesson the user rejected does not come back).
pub fn mark_scanned(c: &Connection, sess: i64, scan: &Scan) -> Result<()> {
    c.execute(
        "UPDATE session SET lessons_seq = MAX(lessons_seq, ?1), lessons_ms = MAX(lessons_ms, ?2) WHERE id = ?3",
        (scan.head_seq, scan.now_ms, sess),
    )?;
    Ok(())
}

/// What [`queue`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Queued {
    pub queued: usize,
    /// Dropped because the pass cap or the waiting cap was reached.
    pub over_cap: usize,
    /// Already remembered or already waiting.
    pub known: usize,
}

/// Put drafts in the approve queue (`memory`'s connection), newest first, up
/// to [`MAX_PER_PASS`] and while fewer than [`MAX_WAITING`] are waiting. Text
/// is masked like every other way a sentence reaches memory (CLAUDE.md 7.28).
/// Lessons never become "replace #id": a lesson is not a better wording of an
/// existing fact.
pub fn queue(mem: &Connection, mut drafts: Vec<LessonDraft>) -> Result<Queued> {
    drafts.sort_by_key(|d| std::cmp::Reverse(d.seq));
    let mut out = Queued::default();
    let mut waiting = memory::pending(mem)?.iter().filter(|p| p.kind == ProposalKind::Lesson).count();
    for d in drafts {
        if out.queued >= MAX_PER_PASS || waiting >= MAX_WAITING {
            out.over_cap += 1;
            continue;
        }
        let text = crate::secrets::user_vault().mask(&d.text).text;
        let evidence = crate::secrets::user_vault().mask(&d.evidence).text;
        let known: bool = mem.query_row(
            "SELECT EXISTS(SELECT 1 FROM fact WHERE text = ?1) OR EXISTS(SELECT 1 FROM fact_proposal WHERE text = ?1)",
            [&text],
            |r| r.get(0),
        )?;
        if known {
            out.known += 1;
            continue;
        }
        memory::enqueue(
            mem,
            &NewProposal {
                kind: ProposalKind::Lesson,
                evidence: Some(&evidence),
                suspicious: d.suspicious,
                ..NewProposal::fact(1, &text)
            },
        )?;
        out.queued += 1;
        waiting += 1;
    }
    Ok(out)
}

/// Ask a local model to say a draft better. Only called when the user set
/// `CHORUS_SUMMARY_MODEL`; any failure, or an answer that is empty, multi-line
/// or over 140 characters, keeps the rule-based sentence.
pub fn polish(model: &crate::compaction::Ollama, draft: &str) -> String {
    let prompt = format!(
        "Rewrite this note about a coding session as ONE plain sentence under 120 characters, in \
         the same language, as advice for a future assistant. Add nothing that is not in it. \
         Reply with the sentence only.\n\n{draft}"
    );
    match model.generate(&prompt) {
        Some(s) => {
            let s = s.trim().trim_matches('"').to_string();
            if s.is_empty() || s.contains('\n') || s.chars().count() > 140 {
                draft.to_string()
            } else {
                s
            }
        }
        None => draft.to_string(),
    }
}

/// Newest user-visible summary of a pass, for CLI and logs.
pub fn describe(q: &Queued) -> String {
    format!("lessons: {} queued · {} already known · {} over cap", q.queued, q.known, q.over_cap)
}
