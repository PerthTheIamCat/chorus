//! A thin C ABI over the Session Engine, for the native shells.
//!
//! Design rule 2.3: no logic lives above this line. Every function here
//! either marshals a call into `chorus_cli::session` or shapes its result
//! as JSON. The Swift (and later C#) side only draws and forwards input.
//!
//! Contract:
//!   - Strings in are borrowed `const char*` (UTF-8, NUL-terminated).
//!   - Strings out are owned `char*` the caller must pass to
//!     `chorus_string_free`. They are always JSON: `{"ok":true,...}` or
//!     `{"ok":false,"error":"..."}`.
//!   - `ChorusEngine*` is opaque; free it with `chorus_engine_free`.
//!   - Every call is blocking. The shell calls off its main thread.

use chorus_cli::session::{Policy, Session};
use chorus_core::handoff::HandoffReason;
use serde::Serialize;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

/// Opaque handle. One per open session.
pub struct ChorusEngine {
    session: Session,
}

// ---- string helpers ---------------------------------------------------

unsafe fn borrow<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn out(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

#[derive(Serialize)]
struct Err_<'a> {
    ok: bool,
    error: &'a str,
}

fn err(msg: impl std::fmt::Display) -> *mut c_char {
    out(serde_json::to_string(&Err_ {
        ok: false,
        error: &msg.to_string(),
    })
    .unwrap())
}

/// Free a string returned by any `chorus_*` function.
///
/// # Safety
/// `p` must be a pointer previously returned by this library, and must not
/// be used again after this call.
#[no_mangle]
pub unsafe extern "C" fn chorus_string_free(p: *mut c_char) {
    if !p.is_null() {
        drop(CString::from_raw(p));
    }
}

// ---- engine lifecycle ----------------------------------------------

/// Open (creating if needed) a store and start a new session.
/// Returns `NULL` on failure.
///
/// # Safety
/// `db_path`, `cwd`, `title` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_engine_open(
    db_path: *const c_char,
    cwd: *const c_char,
    title: *const c_char,
) -> *mut ChorusEngine {
    let run = || -> Option<ChorusEngine> {
        let db = PathBuf::from(borrow(db_path)?);
        let cwd = PathBuf::from(borrow(cwd)?);
        let title = borrow(title)?;
        let session = Session::create(&db, &cwd, title).ok()?;
        Some(ChorusEngine { session })
    };
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Some(e)) => Box::into_raw(Box::new(e)),
        _ => std::ptr::null_mut(),
    }
}

/// Reattach to an existing session by id. Returns `NULL` on failure.
///
/// # Safety
/// `db_path` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_engine_reopen(
    db_path: *const c_char,
    session_id: i64,
) -> *mut ChorusEngine {
    let run = || -> Option<ChorusEngine> {
        let db = PathBuf::from(borrow(db_path)?);
        let session = Session::open(&db, session_id).ok()?;
        Some(ChorusEngine { session })
    };
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Some(e)) => Box::into_raw(Box::new(e)),
        _ => std::ptr::null_mut(),
    }
}

/// Free an engine handle.
///
/// # Safety
/// `e` must be a handle from `chorus_engine_open`/`_reopen`, not freed before.
#[no_mangle]
pub unsafe extern "C" fn chorus_engine_free(e: *mut ChorusEngine) {
    if !e.is_null() {
        drop(Box::from_raw(e));
    }
}

/// The session's numeric id.
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_engine_session_id(e: *const ChorusEngine) -> i64 {
    match e.as_ref() {
        Some(e) => e.session.session_id(),
        None => -1,
    }
}

// ---- turns ---------------------------------------------------------

#[derive(Serialize)]
struct TurnJson {
    ok: bool,
    user_seq: i64,
    agent_event_count: usize,
    message: String,
    context_used: Option<i64>,
    context_window: Option<i64>,
    stop_reason: String,
}

/// Run one turn. `allow` != 0 approves the agent's permission requests.
/// Returns JSON (see `TurnJson` / error shape).
///
/// # Safety
/// `e` must be a live handle; `agent`, `prompt` valid NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_run_turn(
    e: *mut ChorusEngine,
    agent: *const c_char,
    prompt: *const c_char,
    allow: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agent), Some(prompt)) = (borrow(agent), borrow(prompt)) else {
        return err("agent/prompt must be valid UTF-8");
    };
    let policy = Policy::from_level(allow);

    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.run_turn(agent, "openai", prompt, policy)
    }));
    match r {
        Ok(Ok(t)) => out(serde_json::to_string(&TurnJson {
            ok: true,
            user_seq: t.user_seq,
            agent_event_count: t.agent_seqs.len(),
            message: t.message,
            context_used: t.context_used,
            context_window: t.context_window,
            stop_reason: t.stop_reason,
        })
        .unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in run_turn"),
    }
}

/// A permission callback: given the ask as JSON
/// (`{"tool":"...","kind":"...","detail":"..."}`), return non-zero to allow.
/// Called on the same thread as `chorus_run_turn_interactive` (which the
/// shell already runs off its main thread) — the shell hops to its UI,
/// shows a dialog, and blocks here until the user answers.
pub type PermissionCallback =
    extern "C" fn(ctx: *mut std::ffi::c_void, ask_json: *const c_char) -> c_int;

/// Run one turn, routing every permission request to `callback`.
///
/// # Safety
/// `e` live; `agent`/`prompt` valid UTF-8; `callback` a valid function
/// pointer for the duration of the call; `ctx` whatever the callback expects.
#[no_mangle]
pub unsafe extern "C" fn chorus_run_turn_interactive(
    e: *mut ChorusEngine,
    agent: *const c_char,
    prompt: *const c_char,
    callback: PermissionCallback,
    ctx: *mut std::ffi::c_void,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agent), Some(prompt)) = (borrow(agent), borrow(prompt)) else {
        return err("agent/prompt must be valid UTF-8");
    };

    let ctx_usize = ctx as usize; // move a Copy value into the closure
    let mut decide = |ask: &chorus_cli::session::PermissionAsk| {
        let payload = serde_json::json!({
            "tool": ask.tool, "kind": ask.kind, "detail": ask.detail
        })
        .to_string();
        let c = std::ffi::CString::new(payload).unwrap_or_default();
        let allow = callback(ctx_usize as *mut std::ffi::c_void, c.as_ptr());
        if allow != 0 {
            chorus_cli::session::Permission::Allow
        } else {
            chorus_cli::session::Permission::Deny
        }
    };

    let r = catch_unwind(AssertUnwindSafe(|| {
        engine
            .session
            .run_turn_decide(agent, "openai", prompt, &mut decide)
    }));
    match r {
        Ok(Ok(t)) => out(serde_json::to_string(&TurnJson {
            ok: true,
            user_seq: t.user_seq,
            agent_event_count: t.agent_seqs.len(),
            message: t.message,
            context_used: t.context_used,
            context_window: t.context_window,
            stop_reason: t.stop_reason,
        })
        .unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in run_turn_interactive"),
    }
}

// ---- transcript --------------------------------------------------

#[derive(Serialize)]
struct EventJson {
    seq: i64,
    kind: &'static str,
    actor: String,
    body: String,
}

/// The whole session so far as a JSON array of events.
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_transcript_json(e: *const ChorusEngine) -> *mut c_char {
    chorus_events_since(e, 0)
}

/// Only the events with `seq` greater than `after` — the delta a UI polls
/// so it pulls just what it hasn't rendered. Pass 0 for the whole session.
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_events_since(e: *const ChorusEngine, after: i64) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let r = catch_unwind(AssertUnwindSafe(|| engine.session.events_since(after)));
    match r {
        Ok(Ok(rows)) => {
            let events: Vec<EventJson> = rows
                .into_iter()
                .map(|(seq, kind, actor, body)| EventJson {
                    seq,
                    kind,
                    actor,
                    body,
                })
                .collect();
            out(serde_json::to_string(&events).unwrap())
        }
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in transcript"),
    }
}

// ---- checkpoints -----------------------------------------------

/// The workspace diff since the session base checkpoint.
/// JSON: {"ok":true,"diff":"..."}
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_working_diff(e: *const ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let diff = catch_unwind(AssertUnwindSafe(|| engine.session.working_diff())).unwrap_or_default();
    out(serde_json::json!({ "ok": true, "diff": diff }).to_string())
}

/// Take a manual checkpoint. JSON: {"ok":true,"sha":"..."}
///
/// # Safety
/// `e` live; `message` valid NUL-terminated UTF-8 or NULL.
#[no_mangle]
pub unsafe extern "C" fn chorus_snapshot(
    e: *const ChorusEngine,
    message: *const c_char,
) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let msg = borrow(message).unwrap_or("manual checkpoint");
    match catch_unwind(AssertUnwindSafe(|| engine.session.snapshot(msg))) {
        Ok(Ok(sha)) => out(serde_json::json!({ "ok": true, "sha": sha }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in snapshot"),
    }
}

/// Reject a hunk selection: reverse-apply `patch` (only the rejected
/// hunks) to the workspace. JSON: {"ok":true}
///
/// # Safety
/// `e` live; `patch` valid NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_reject_hunks(
    e: *const ChorusEngine,
    patch: *const c_char,
) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let Some(patch) = borrow(patch) else {
        return err("patch must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| engine.session.reject_hunks(patch))) {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in reject_hunks"),
    }
}

/// Roll the workspace back to `sha` (empty = the session base). Destructive.
/// JSON: {"ok":true}
///
/// # Safety
/// `e` live; `sha` valid NUL-terminated UTF-8 or NULL.
#[no_mangle]
pub unsafe extern "C" fn chorus_restore(e: *const ChorusEngine, sha: *const c_char) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let sha = borrow(sha).unwrap_or("");
    match catch_unwind(AssertUnwindSafe(|| engine.session.restore_to(sha))) {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in restore"),
    }
}

// ---- memory ----------------------------------------------------

/// The memory inspector's data: approved facts + the pending proposal
/// queue. JSON: {"ok":true,"facts":["...", ...],
///               "pending":[{"id":N,"scope":N,"text":"..."}, ...]}
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_memory_state(e: *const ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let r = catch_unwind(AssertUnwindSafe(|| engine.session.memory_state()));
    match r {
        Ok(Ok((facts, pending))) => out(serde_json::json!({
            "ok": true,
            "facts": facts,
            "pending": pending.iter().map(|(id, scope, text)| {
                serde_json::json!({ "id": id, "scope": scope, "text": text })
            }).collect::<Vec<_>>(),
        })
        .to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in memory_state"),
    }
}

/// Approve (`decision` != 0) or reject a proposal by id. JSON: {"ok":true}
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_memory_review(
    e: *const ChorusEngine,
    proposal_id: i64,
    approve: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.review_proposal(proposal_id, approve != 0)
    }));
    match r {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in memory_review"),
    }
}

// ---- handoff ----------------------------------------------------

#[derive(Serialize)]
struct HandoffJson {
    ok: bool,
    handoff_seq: i64,
    from: String,
    to: String,
    brief_markdown: String,
    brief_json: String,
    echo_back: String,
    stop_reason: String,
}

/// Hand the session to `to`, building the brief from the ledger. `goal` is
/// required; `done_json` / `pending_json` are JSON string arrays (or NULL).
///
/// # Safety
/// `e` live; string args valid NUL-terminated UTF-8 or NULL where allowed.
#[no_mangle]
pub unsafe extern "C" fn chorus_handoff(
    e: *mut ChorusEngine,
    to: *const c_char,
    reason: *const c_char,
    goal: *const c_char,
    done_json: *const c_char,
    pending_json: *const c_char,
    allow: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(to), Some(goal)) = (borrow(to), borrow(goal)) else {
        return err("to/goal must be valid UTF-8");
    };
    let reason = parse_reason(borrow(reason).unwrap_or("manual"));
    let done = parse_str_array(borrow(done_json));
    let pending = parse_str_array(borrow(pending_json));
    let policy = Policy::from_level(allow);

    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.handoff(
            to,
            "openai",
            reason,
            goal,
            done,
            pending,
            Vec::new(),
            String::new(),
            policy,
        )
    }));
    match r {
        Ok(Ok(h)) => out(serde_json::to_string(&HandoffJson {
            ok: true,
            handoff_seq: h.handoff_seq,
            from: h.from,
            to: h.to,
            brief_markdown: h.brief.render_markdown(),
            brief_json: h.brief.to_json(),
            echo_back: h.echo_back,
            stop_reason: h.stop_reason,
        })
        .unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in handoff"),
    }
}

// ---- relay + compare --------------------------------------------

fn specs_from_csv(csv: &str) -> Vec<chorus_cli::session::AgentSpec> {
    csv.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|name| {
            let provider = if name.contains("claude") {
                "anthropic"
            } else if name.contains("gemini") {
                "google"
            } else {
                "openai"
            };
            chorus_cli::session::AgentSpec::new(name, provider)
        })
        .collect()
}

#[derive(Serialize)]
struct RelayJson {
    ok: bool,
    answered_by: String,
    message: String,
    stop_reason: String,
    handoffs: Vec<[String; 3]>,
}

/// Run one turn against a relay chain (`agents` = comma-separated names).
///
/// # Safety
/// `e` live; `agents`, `prompt` valid NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_run_relay(
    e: *mut ChorusEngine,
    agents: *const c_char,
    prompt: *const c_char,
    allow: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agents), Some(prompt)) = (borrow(agents), borrow(prompt)) else {
        return err("agents/prompt must be valid UTF-8");
    };
    let specs = specs_from_csv(agents);
    if specs.len() < 2 {
        return err("relay needs at least two agents");
    }
    let policy = Policy::from_level(allow);

    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.run_relay(&specs, prompt, policy)
    }));
    match r {
        Ok(Ok(rr)) => out(serde_json::to_string(&RelayJson {
            ok: true,
            answered_by: rr.answered_by,
            message: rr.turn.message,
            stop_reason: rr.turn.stop_reason,
            handoffs: rr
                .handoffs
                .into_iter()
                .map(|(f, t, why)| [f, t, why.as_phrase().to_string()])
                .collect(),
        })
        .unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in run_relay"),
    }
}

#[derive(Serialize)]
struct CompareJson {
    ok: bool,
    branches: Vec<CompareBranchJson>,
}
#[derive(Serialize)]
struct CompareBranchJson {
    agent: String,
    session_id: i64,
    message: String,
    stop_reason: String,
    error: Option<String>,
}

/// Compare mode: fork per agent, run `prompt` on each, return each branch.
///
/// # Safety
/// `e` live; `agents`, `prompt` valid NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_compare(
    e: *mut ChorusEngine,
    agents: *const c_char,
    prompt: *const c_char,
    allow: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agents), Some(prompt)) = (borrow(agents), borrow(prompt)) else {
        return err("agents/prompt must be valid UTF-8");
    };
    let specs = specs_from_csv(agents);
    if specs.len() < 2 {
        return err("compare needs at least two agents");
    }
    let policy = Policy::from_level(allow);

    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.compare(&specs, prompt, policy)
    }));
    match r {
        Ok(Ok(branches)) => out(serde_json::to_string(&CompareJson {
            ok: true,
            branches: branches
                .into_iter()
                .map(|b| match b.result {
                    Ok((message, stop_reason)) => CompareBranchJson {
                        agent: b.agent,
                        session_id: b.session_id,
                        message,
                        stop_reason,
                        error: None,
                    },
                    Err(e) => CompareBranchJson {
                        agent: b.agent,
                        session_id: b.session_id,
                        message: String::new(),
                        stop_reason: String::new(),
                        error: Some(format!("{e:#}")),
                    },
                })
                .collect(),
        })
        .unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in compare"),
    }
}

fn parse_str_array(s: Option<&str>) -> Vec<String> {
    s.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

fn parse_reason(s: &str) -> HandoffReason {
    match s.replace(['-', ' '], "_").to_lowercase().as_str() {
        "rate_limit" => HandoffReason::RateLimit,
        "quota_exhausted" | "quota" => HandoffReason::QuotaExhausted,
        "context_exhausted" | "context" => HandoffReason::ContextExhausted,
        "user_requested" | "user" => HandoffReason::UserRequested,
        "failover" => HandoffReason::Failover,
        _ => HandoffReason::Manual,
    }
}

/// Stop the turn that is running right now (the shell's Stop button).
/// Safe to call from any thread, with or without a live engine.
/// JSON: {"ok":true,"was_running":bool}
#[no_mangle]
pub extern "C" fn chorus_cancel() -> *mut c_char {
    out(serde_json::json!({ "ok": true, "was_running": chorus_cli::cancel_active() }).to_string())
}

/// Point this session's memory (facts + approve queue + MCP `memory_propose`)
/// at a shared project store. The folder's own facts are copied in once.
/// JSON: {"ok":true}
///
/// # Safety
/// `e` must be a live handle; `path` a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_set_memory_db(
    e: *mut ChorusEngine,
    path: *const c_char,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let Some(path) = borrow(path) else {
        return err("path must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        engine.session.set_memory_db(std::path::Path::new(path))
    })) {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in set_memory_db"),
    }
}

/// Live view of the running turn. `cb(ctx, kind, text)` is called from the
/// turn's thread for each update: kind is `thought` | `message` | `tool` |
/// `tool_done`. Both strings are only valid during the call. Pass a null `cb`
/// to stop. Display only: the ledger is unaffected.
///
/// # Safety
/// `ctx` must stay valid until the callback is replaced or cleared.
#[no_mangle]
pub unsafe extern "C" fn chorus_set_progress_callback(
    cb: Option<extern "C" fn(*mut std::ffi::c_void, *const c_char, *const c_char)>,
    ctx: *mut std::ffi::c_void,
) {
    let Some(cb) = cb else {
        chorus_cli::set_progress_sink(None);
        return;
    };
    let ctx = ctx as usize;
    chorus_cli::set_progress_sink(Some(Box::new(move |kind: &str, text: &str| {
        let (Ok(k), Ok(t)) = (CString::new(kind), CString::new(text.replace('\0', ""))) else {
            return;
        };
        cb(ctx as *mut std::ffi::c_void, k.as_ptr(), t.as_ptr());
    })));
}

/// How many events still have no stored summary rungs. The shell polls this
/// to decide whether a background compaction pass is worth running.
/// JSON: {"ok":true,"pending":N}
///
/// # Safety
/// `e` must be a live handle from `chorus_engine_open`/`_reopen`.
#[no_mangle]
pub unsafe extern "C" fn chorus_uncompacted(e: *const ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    match catch_unwind(AssertUnwindSafe(|| engine.session.uncompacted_count())) {
        Ok(Ok(n)) => out(serde_json::json!({ "ok": true, "pending": n }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in uncompacted"),
    }
}

/// Fill in up to `limit` missing summary rungs (the warm tier).
///
/// **Call this off the UI thread and off the turn path.** It is allowed to be
/// slow — it may run a local model per event — which is exactly why context
/// assembly never does this work itself.
/// JSON: {"ok":true,"compacted":N,"remaining":N,"model":"…"}
///
/// # Safety
/// `e` must be a live handle from `chorus_engine_open`/`_reopen`.
#[no_mangle]
pub unsafe extern "C" fn chorus_compact(e: *const ChorusEngine, limit: i32) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let limit = if limit <= 0 { 200 } else { limit as usize };
    match catch_unwind(AssertUnwindSafe(|| engine.session.compact(limit))) {
        Ok(Ok(r)) => out(serde_json::json!({
            "ok": true,
            "compacted": r.compacted,
            "remaining": r.remaining,
            "model": r.model,
            "took_ms": r.took_ms,
            // True when the pass stopped on the clock rather than finishing.
            // Not an error: it is how a slow summariser is meant to behave.
            "hit_budget": r.hit_budget,
        })
        .to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in compact"),
    }
}

/// The task-state tier for this session — goal, done, pending, constraints.
/// JSON: {"ok":true,"goal":"…","done":[…],"pending":[…],"constraints":[…],"text":"…"}
///
/// # Safety
/// `e` must be a live handle from `chorus_engine_open`/`_reopen`.
#[no_mangle]
pub unsafe extern "C" fn chorus_task_state(e: *const ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    match catch_unwind(AssertUnwindSafe(|| engine.session.task_state())) {
        Ok(Ok(t)) => out(serde_json::json!({
            "ok": true,
            "goal": t.goal,
            "done": t.done,
            "pending": t.pending,
            "constraints": t.constraints,
            "text": t.render(),
        })
        .to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in task_state"),
    }
}

/// The context Chorus would hand an agent right now, assembled from the
/// ledger exactly as a real turn would.
///
/// This is the app's honesty surface: it is not a description of what the
/// agent sees, it *is* what the agent sees. JSON:
/// {"ok":true,"text":"…","tokens":N,"budget":N}
///
/// # Safety
/// `e` must be a live handle from `chorus_engine_open`/`_reopen`.
#[no_mangle]
pub unsafe extern "C" fn chorus_assembled_context(
    e: *const ChorusEngine,
    budget: i64,
) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let budget = if budget <= 0 { 120_000 } else { budget };
    match catch_unwind(AssertUnwindSafe(|| engine.session.assemble_context(budget))) {
        Ok(Ok(text)) => {
            use chorus_core::tokens::{CachedCounter, HeuristicCounter, TokenCounter};
            let tokens = CachedCounter::new(HeuristicCounter).count(&text);
            out(serde_json::json!({
                "ok": true, "text": text, "tokens": tokens, "budget": budget
            })
            .to_string())
        }
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in assembled_context"),
    }
}

// ---- session picker + agent inventory ---------------------------

#[derive(Serialize)]
struct SessionSummaryJson {
    id: i64,
    title: String,
    cwd: String,
    started_ms: i64,
    last_ms: i64,
    events: i64,
    agents: Vec<String>,
    goal: String,
    forked_from: Option<i64>,
}

/// Every session in `db_path`, newest activity first — the shell's session
/// picker. Does not need (or open) a `ChorusEngine`: this is a store-level
/// query, one connection for however many sessions exist.
/// JSON: {"ok":true,"sessions":[{"id":N,"title":"...","cwd":"...",
///        "started_ms":N,"last_ms":N,"events":N,"agents":["..."],
///        "goal":"...","forked_from":N|null}, ...]}
///
/// # Safety
/// `db_path` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_list_sessions(db_path: *const c_char) -> *mut c_char {
    let Some(db_path) = borrow(db_path) else {
        return err("db_path must be valid UTF-8");
    };
    let db = PathBuf::from(db_path);
    let r = catch_unwind(AssertUnwindSafe(|| chorus_cli::session::list_sessions(&db)));
    match r {
        Ok(Ok(rows)) => {
            let sessions: Vec<SessionSummaryJson> = rows
                .into_iter()
                .map(|s| SessionSummaryJson {
                    id: s.id,
                    title: s.title,
                    cwd: s.cwd,
                    started_ms: s.started_ms,
                    last_ms: s.last_ms,
                    events: s.events,
                    agents: s.agents,
                    goal: s.goal,
                    forked_from: s.forked_from,
                })
                .collect();
            out(serde_json::json!({ "ok": true, "sessions": sessions }).to_string())
        }
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in list_sessions"),
    }
}

#[derive(Serialize)]
struct ModelChoiceJson {
    id: String,
    label: String,
    is_default: bool,
}

#[derive(Serialize)]
struct AgentInfoJson {
    id: String,
    label: String,
    models: Vec<ModelChoiceJson>,
    auth: &'static str,
    model_source: &'static str,
    hint: String,
    detail: String,
}

fn agent_info_json(a: chorus_cli::agents::AgentInfo) -> AgentInfoJson {
    AgentInfoJson {
        id: a.id,
        label: a.label,
        models: a
            .models
            .into_iter()
            .map(|m| ModelChoiceJson {
                id: m.id,
                label: m.label,
                is_default: m.is_default,
            })
            .collect(),
        auth: a.auth.as_str(),
        model_source: a.model_source.as_str(),
        hint: a.hint,
        detail: a.detail,
    }
}

/// Pin the model an agent should run, by the id it reports itself
/// (`chorus_list_agents` → `models[].id`). Empty `model` clears the pin.
///
/// Session-scoped and applied when the next turn opens its ACP session, so
/// calling it between turns is enough — and calling it *is* required: without
/// it a model picker only changes a label.
///
/// # Safety
/// `agent` and `model` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_set_model(
    e: *mut ChorusEngine,
    agent: *const c_char,
    model: *const c_char,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agent), Some(model)) = (borrow(agent), borrow(model)) else {
        return err("agent/model must be valid UTF-8");
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.set_model(agent, model);
    }));
    match r {
        Ok(()) => out(serde_json::json!({ "ok": true }).to_string()),
        Err(_) => err("panic in set_model"),
    }
}

/// Every skill an agent in this workspace could see, grouped by where it lives:
/// `chorus` (`.chorus/skills`, projected to every agent), `project`
/// (`.claude/skills`) and `user` (`~/.claude/skills`). Read-only.
/// JSON: {"ok":true,"skills":[{"name","description","path","source"}]}
///
/// # Safety
/// `cwd` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_list_skills(cwd: *const c_char) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else {
        return err("cwd must be valid UTF-8");
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        let cwd = std::path::Path::new(cwd);
        let mut roots = vec![
            ("chorus", cwd.join(chorus_core::skills::SKILLS_DIR)),
            ("project", cwd.join(chorus_core::skills::CLAUDE_SKILLS_DIR)),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(("user", std::path::Path::new(&home).join(".claude/skills")));
        }
        let mut skills = Vec::new();
        for (source, root) in roots {
            for (name, description, dir) in chorus_core::skills::list_dir(&root) {
                skills.push(serde_json::json!({
                    "name": name, "description": description,
                    "path": dir.join("SKILL.md").to_string_lossy(), "source": source,
                }));
            }
        }
        skills
    }));
    match r {
        Ok(skills) => out(serde_json::json!({ "ok": true, "skills": skills }).to_string()),
        Err(_) => err("panic in list_skills"),
    }
}

/// Rename a session. JSON: {"ok":true}
///
/// # Safety
/// `db_path` and `title` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_rename_session(
    db_path: *const c_char,
    id: i64,
    title: *const c_char,
) -> *mut c_char {
    let (Some(db), Some(title)) = (borrow(db_path), borrow(title)) else {
        return err("db_path/title must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        chorus_cli::session::rename_session(&PathBuf::from(db), id, title)
    })) {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in rename_session"),
    }
}

/// Delete a session and its events. Refused if another session forked from
/// it. Close the session first if it is open. JSON: {"ok":true}
///
/// # Safety
/// `db_path` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_delete_session(db_path: *const c_char, id: i64) -> *mut c_char {
    let Some(db) = borrow(db_path) else {
        return err("db_path must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        chorus_cli::session::delete_session(&PathBuf::from(db), id)
    })) {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in delete_session"),
    }
}

/// Ask an agent for its current model list (opens a real ACP session in
/// `cwd`, so it is slow — call off the UI thread) and refresh the cache that
/// `chorus_list_agents` reads. JSON: {"ok":true,"count":N}
///
/// # Safety
/// `agent_id` and `cwd` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_refresh_models(
    agent_id: *const c_char,
    cwd: *const c_char,
) -> *mut c_char {
    let (Some(agent_id), Some(cwd)) = (borrow(agent_id), borrow(cwd)) else {
        return err("agent_id/cwd must be valid UTF-8");
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        chorus_cli::agents::refresh_models(agent_id, std::path::Path::new(cwd))
    }));
    match r {
        Ok(Ok(m)) => out(serde_json::json!({ "ok": true, "count": m.len() }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in refresh_models"),
    }
}

/// Every agent Chorus knows how to bridge to, with whether it can run right
/// now. Cheap and non-blocking — no login, no network (see `chorus_cli::agents`).
/// JSON: {"ok":true,"agents":[{"id":"...","label":"...",
///        "models":[{"id":"...","label":"...","is_default":true}, ...],
///        "auth":"ok"|"needs_login"|"missing_cli",
///        "model_source":"probed"|"env"|"fallback","hint":"...","detail":"..."}, ...]}
#[no_mangle]
pub extern "C" fn chorus_list_agents() -> *mut c_char {
    let r = catch_unwind(AssertUnwindSafe(chorus_cli::agents::list_agents));
    match r {
        Ok(agents) => {
            let agents: Vec<AgentInfoJson> = agents.into_iter().map(agent_info_json).collect();
            out(serde_json::json!({ "ok": true, "agents": agents }).to_string())
        }
        Err(_) => err("panic in list_agents"),
    }
}

/// One agent's current runnability, by id ("claude" | "codex"). Same shape
/// as one entry of `chorus_list_agents`.
///
/// # Safety
/// `agent_id` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_auth_status(agent_id: *const c_char) -> *mut c_char {
    let Some(agent_id) = borrow(agent_id) else {
        return err("agent_id must be valid UTF-8");
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        chorus_cli::agents::auth_status(agent_id)
    }));
    match r {
        Ok(a) => out(serde_json::to_string(&agent_info_json(a)).unwrap()),
        Err(_) => err("panic in auth_status"),
    }
}

#[derive(Serialize)]
struct HandoffAutoJson {
    ok: bool,
    to: String,
    goal: String,
    echo_back: String,
}

/// One-click handoff: goal/done/pending/constraints all come from the
/// session's own `task_state()` — the caller passes nothing but `to` and a
/// permission `level`. `level` maps through `Policy::from_level`
/// (0 deny · 1 read-only · 2 allow).
/// JSON: {"ok":true,"to":"...","goal":"...","echo_back":"..."}
///
/// # Safety
/// `e` must be a live engine handle; `to` a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_handoff_auto(
    e: *mut ChorusEngine,
    to: *const c_char,
    level: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let Some(to) = borrow(to) else {
        return err("to must be valid UTF-8");
    };
    let policy = Policy::from_level(level);

    let r = catch_unwind(AssertUnwindSafe(|| {
        engine.session.handoff_auto(to, "openai", policy)
    }));
    match r {
        Ok(Ok(h)) => out(serde_json::to_string(&HandoffAutoJson {
            ok: true,
            to: h.to,
            goal: h.brief.goal.clone(),
            echo_back: h.echo_back,
        })
        .unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in handoff_auto"),
    }
}
