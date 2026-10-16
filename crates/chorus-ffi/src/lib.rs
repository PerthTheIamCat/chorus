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
use chorus_core::attachments::{Attachment, MAX_FILE_BYTES};
use chorus_core::handoff::HandoffReason;
use chorus_core::integration_catalog;
use chorus_core::router::{CandidateSet, Router, RouterConfig, TaskSummary};
use serde::Serialize;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

/// Curated catalog and user/project state, without reading any secret values.
/// A project path is used only to scope enabled integrations.
///
/// # Safety
/// `project` must point to a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_catalog_list_json(project: *const c_char) -> *mut c_char {
    let Some(project) = borrow(project) else { return err("project must be valid UTF-8") };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let state = integration_catalog::load_state().map_err(|e| e.to_string())?;
        let enabled = integration_catalog::enabled_for(Path::new(project)).map_err(|e| e.to_string())?;
        let enabled: std::collections::BTreeSet<&str> = enabled.iter().map(|entry| entry.id).collect();
        let entries: Vec<_> = integration_catalog::catalog().iter().map(|entry| {
            let installed = integration_catalog::current_record(entry, &state);
            let installed = installed.as_ref();
            serde_json::json!({
                "id": entry.id, "title": entry.title, "source": entry.source,
                "maintainer": entry.maintainer, "license": entry.license,
                "reviewed_version": entry.reviewed_version, "modes": entry.modes,
                "required_runtimes": entry.required_runtimes, "actions": entry.actions,
                "project_access": entry.project_access, "network": entry.network,
                "secret_names": entry.secret_names, "installed": installed,
                "installed_method": installed.map(|record| record.method),
                "brew_formula": (entry.recipe.method == integration_catalog::InstallMethod::Homebrew).then_some(entry.recipe.package),
                "missing_installer": integration_catalog::missing_installer(entry),
                "enabled": enabled.contains(entry.id),
                "can_remove": true,
            })
        }).collect();
        Ok(serde_json::json!({"ok":true,"entries":entries}))
    }));
    match result {
        Ok(Ok(value)) => out(value.to_string()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in catalog list"),
    }
}

/// `action`: install, update, remove, enable, or disable. Install/update use
/// Chorus-owned paths and the catalog's approved recipe.
///
/// # Safety
/// All pointers must point to valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_catalog_action_json(
    project: *const c_char, id: *const c_char, action: *const c_char,
) -> *mut c_char {
    let (Some(project), Some(id), Some(action)) = (borrow(project), borrow(id), borrow(action)) else {
        return err("catalog action arguments must be valid UTF-8");
    };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        integration_catalog::perform(action, id, Path::new(project)).map_err(|e| e.to_string())
    }));
    match result {
        Ok(Ok(())) => out("{\"ok\":true}".to_string()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in catalog action"),
    }
}

/// What `action` would create, change or remove. Same arguments as
/// `chorus_catalog_action_json`; nothing is touched.
///
/// # Safety
/// All pointers must point to valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_catalog_plan_json(
    project: *const c_char, id: *const c_char, action: *const c_char,
) -> *mut c_char {
    let (Some(project), Some(id), Some(action)) = (borrow(project), borrow(id), borrow(action)) else {
        return err("catalog plan arguments must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| integration_catalog::plan(id, action, Path::new(project)))) {
        Ok(Ok(plan)) => out(serde_json::json!({"ok":true,"plan":plan}).to_string()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in catalog plan"),
    }
}

/// Dashboard data for one installed integration, scoped to `project`.
///
/// # Safety
/// All pointers must point to valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_catalog_dashboard_json(project: *const c_char, id: *const c_char) -> *mut c_char {
    let (Some(project), Some(id)) = (borrow(project), borrow(id)) else {
        return err("dashboard arguments must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| integration_catalog::dashboard(id, Path::new(project)))) {
        Ok(Ok(d)) => out(serde_json::json!({"ok":true,"dashboard":d}).to_string()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in catalog dashboard"),
    }
}

/// Run one of an integration's `run` actions. The shell must have shown the
/// action's `effects` and got a yes first.
///
/// # Safety
/// All pointers must point to valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_catalog_run_json(project: *const c_char, id: *const c_char, key: *const c_char) -> *mut c_char {
    let (Some(project), Some(id), Some(key)) = (borrow(project), borrow(id), borrow(key)) else {
        return err("run arguments must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| integration_catalog::run_action(id, key, Path::new(project)))) {
        Ok(Ok(result)) => out(serde_json::json!({"ok":true,"result":result}).to_string()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in catalog run"),
    }
}

/// Position of the running catalog action, or `{"ok":true,"progress":null}`.
#[no_mangle]
pub extern "C" fn chorus_catalog_progress_json() -> *mut c_char {
    out(serde_json::json!({"ok":true,"progress":integration_catalog::progress()}).to_string())
}

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

unsafe fn borrow_optional<'a>(p: *const c_char) -> Result<Option<&'a str>, &'static str> {
    if p.is_null() {
        return Ok(None);
    }
    CStr::from_ptr(p)
        .to_str()
        .map(Some)
        .map_err(|_| "string must be valid UTF-8")
}

unsafe fn borrow_bytes<'a>(p: *const u8, len: usize) -> Option<&'a [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if p.is_null() || len > isize::MAX as usize {
        return None;
    }
    Some(std::slice::from_raw_parts(p, len))
}

unsafe fn borrow_attachment_ids(p: *const c_char) -> Result<Vec<i64>, &'static str> {
    let json = borrow(p).ok_or("attachment_ids must be a non-null UTF-8 JSON array")?;
    if json.len() > 1024 {
        return Err("attachment_ids JSON exceeds 1024 bytes");
    }
    let ids: Vec<i64> = serde_json::from_str(json)
        .map_err(|_| "attachment_ids must be a JSON array of integers")?;
    if ids.len() > 5 {
        return Err("at most five attachments may be sent in one message");
    }
    Ok(ids)
}

/// Run the local-only router against JSON task/candidate inputs.
///
/// # Safety
/// All three arguments must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_route_json(
    task_json: *const c_char,
    candidates_json: *const c_char,
    model: *const c_char,
) -> *mut c_char {
    let (Some(task_json), Some(candidates_json), Some(model)) =
        (borrow(task_json), borrow(candidates_json), borrow(model))
    else {
        return err("task_json, candidates_json, and model must be valid UTF-8");
    };
    if task_json.len() > 512 * 1024 || candidates_json.len() > 512 * 1024 {
        return err("router JSON inputs must each be at most 512 KiB");
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let task: TaskSummary =
            serde_json::from_str(task_json).map_err(|e| format!("invalid task_json: {e}"))?;
        let candidates: CandidateSet = serde_json::from_str(candidates_json)
            .map_err(|e| format!("invalid candidates_json: {e}"))?;
        let router = Router::new(RouterConfig::local(model))
            .map_err(|e| format!("invalid router configuration: {e}"))?;
        Ok::<_, String>(router.route_with_fallback(&task, &candidates))
    })) {
        Ok(Ok(route)) => out(serde_json::json!({"ok":true,"route":route}).to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in route_json"),
    }
}

/// Discover locally installed Ollama models for router setup.
///
/// # Safety
/// `model` must point to a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_router_models_json(model: *const c_char) -> *mut c_char {
    let Some(model) = borrow(model) else {
        return err("model must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        let router = Router::new(RouterConfig::local(model)).map_err(|error| error.to_string())?;
        router.discover_models().map_err(|error| error.message)
    })) {
        Ok(Ok(models)) => out(serde_json::json!({"ok":true,"models":models}).to_string()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in router model discovery"),
    }
}

/// Pull one named model from local Ollama for router setup.
///
/// # Safety
/// `model` must point to a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_router_pull_json(model: *const c_char) -> *mut c_char {
    let Some(model) = borrow(model) else {
        return err("model must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        let router = Router::new(RouterConfig::local(model)).map_err(|error| error.to_string())?;
        router.pull_model(model)
    })) {
        Ok(Ok(())) => out("{\"ok\":true}".to_owned()),
        Ok(Err(error)) => err(error),
        Err(_) => err("panic in router model pull"),
    }
}

/// List curated local models and their installation/disk status.
#[no_mangle]
pub unsafe extern "C" fn chorus_model_manager_list_json() -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(|| {
        let mgr = chorus_core::model_manager::ModelManager::new();
        let models: Vec<_> = chorus_core::model_manager::ModelManager::curated_models()
            .into_iter()
            .map(|spec| mgr.status(&spec))
            .collect();
        serde_json::to_string(&serde_json::json!({
            "ok": true,
            "models": models
        }))
    })) {
        Ok(Ok(json)) => out(json),
        Ok(Err(e)) => err(e.to_string()),
        Err(_) => err("panic in model_manager_list"),
    }
}

/// Finalize a downloaded temp file into Chorus local models storage.
///
/// # Safety
/// `temp_path` and `model_id` must point to valid UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_model_manager_finalize_json(
    temp_path: *const c_char,
    model_id: *const c_char,
) -> *mut c_char {
    let Some(temp_path) = borrow(temp_path) else {
        return err("temp_path must be valid UTF-8");
    };
    let Some(model_id) = borrow(model_id) else {
        return err("model_id must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        let mgr = chorus_core::model_manager::ModelManager::new();
        let spec = chorus_core::model_manager::ModelManager::find_spec(model_id)
            .ok_or_else(|| chorus_core::error::ChorusError::Invalid(format!("unknown model id: {model_id}")))?;
        let path = mgr.finalize_download(std::path::Path::new(temp_path), &spec)?;
        Ok::<_, chorus_core::error::ChorusError>(serde_json::json!({
            "ok": true,
            "path": path.to_string_lossy().to_string()
        }).to_string())
    })) {
        Ok(Ok(json)) => out(json),
        Ok(Err(e)) => err(e.to_string()),
        Err(_) => err("panic in model_manager_finalize"),
    }
}

/// Delete a model file from disk to reclaim storage.
///
/// # Safety
/// `model_id` must point to a valid UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn chorus_model_manager_delete_json(
    model_id: *const c_char,
) -> *mut c_char {
    let Some(model_id) = borrow(model_id) else {
        return err("model_id must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        let mgr = chorus_core::model_manager::ModelManager::new();
        let spec = chorus_core::model_manager::ModelManager::find_spec(model_id)
            .ok_or_else(|| chorus_core::error::ChorusError::Invalid(format!("unknown model id: {model_id}")))?;
        let deleted = mgr.delete_model(&spec)?;
        Ok::<_, chorus_core::error::ChorusError>(serde_json::json!({
            "ok": true,
            "deleted": deleted
        }).to_string())
    })) {
        Ok(Ok(json)) => out(json),
        Ok(Err(e)) => err(e.to_string()),
        Err(_) => err("panic in model_manager_delete"),
    }
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

// ---- attachments -----------------------------------------------------

/// Import original bytes and optional locally prepared text or image data.
/// The prepared values are mutually exclusive. `normalized_image_mime == NULL`
/// means there is no normalized image; `derived_text == NULL` means no text.
///
/// # Safety
/// `e` must be live; `name` valid UTF-8; each non-empty byte range must be
/// readable for its stated length; optional strings must be valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn chorus_import_attachment(
    e: *mut ChorusEngine,
    name: *const c_char,
    media_type: *const c_char,
    bytes: *const u8,
    bytes_len: usize,
    derived_text: *const c_char,
    normalized_image: *const u8,
    normalized_image_len: usize,
    normalized_image_mime: *const c_char,
) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    let Some(name) = borrow(name) else {
        return err("name must be valid UTF-8");
    };
    let media_type = match borrow_optional(media_type) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let derived_text = match borrow_optional(derived_text) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let normalized_image_mime = match borrow_optional(normalized_image_mime) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if bytes_len > MAX_FILE_BYTES {
        return err("attachment exceeds 20 MiB");
    }
    if normalized_image_len > MAX_FILE_BYTES {
        return err("normalized image exceeds 20 MiB");
    }
    if derived_text.is_some_and(|s| s.len() > MAX_FILE_BYTES) {
        return err("prepared text exceeds 20 MiB");
    }
    if (normalized_image_mime.is_some()) != (normalized_image_len > 0)
        || (normalized_image_len > 0 && normalized_image.is_null())
        || (normalized_image_mime.is_none() && !normalized_image.is_null())
    {
        return err("normalized image MIME type and bytes must be provided together");
    }
    let Some(bytes) = borrow_bytes(bytes, bytes_len) else {
        return err("invalid attachment byte pointer or length");
    };
    let normalized_image_bytes = if normalized_image_mime.is_some() {
        let Some(data) = borrow_bytes(normalized_image, normalized_image_len) else {
            return err("invalid normalized image byte pointer or length");
        };
        Some(data)
    } else {
        None
    };

    match catch_unwind(AssertUnwindSafe(|| {
        engine.session.import_attachment(
            name,
            media_type,
            bytes,
            derived_text,
            normalized_image_mime.zip(normalized_image_bytes),
        )
    })) {
        Ok(Ok(a)) => {
            out(serde_json::json!({"ok":true,"attachment":attachment_json(&a)}).to_string())
        }
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in import_attachment"),
    }
}

/// List this session's staged, sent, and deleted attachment metadata.
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_list_attachments(e: *const ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    match catch_unwind(AssertUnwindSafe(|| engine.session.list_attachments())) {
        Ok(Ok(rows)) => {
            let attachments: Vec<_> = rows.iter().map(attachment_json).collect();
            out(serde_json::json!({"ok":true,"attachments":attachments}).to_string())
        }
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in list_attachments"),
    }
}

/// Tombstone an attachment owned by this session.
///
/// # Safety
/// `e` must be a live engine handle.
#[no_mangle]
pub unsafe extern "C" fn chorus_delete_attachment(
    e: *const ChorusEngine,
    attachment_id: i64,
) -> *mut c_char {
    let Some(engine) = e.as_ref() else {
        return err("null engine");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        engine.session.delete_attachment(attachment_id)
    })) {
        Ok(Ok(())) => out(serde_json::json!({"ok":true}).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in delete_attachment"),
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

#[derive(Serialize)]
struct AttachmentJson<'a> {
    id: i64,
    name: &'a str,
    media_type: &'a str,
    size: i64,
    status: &'a str,
    readable: bool,
    event_seq: Option<i64>,
}

fn attachment_json(a: &Attachment) -> AttachmentJson<'_> {
    let readable = a.status != "deleted"
        && (a.prepared_hash.is_some()
            || a.media_type.starts_with("image/")
            || a.media_type.starts_with("text/")
            || a.media_type == "application/json"
            || a.media_type.ends_with("+json")
            || a.media_type.ends_with("+xml"));
    AttachmentJson {
        id: a.id,
        name: &a.name,
        media_type: &a.media_type,
        size: a.size,
        status: &a.status,
        readable,
        event_seq: a.event_seq,
    }
}

fn turn_json(t: chorus_cli::session::TurnReport) -> TurnJson {
    TurnJson {
        ok: true,
        user_seq: t.user_seq,
        agent_event_count: t.agent_seqs.len(),
        message: t.message,
        context_used: t.context_used,
        context_window: t.context_window,
        stop_reason: t.stop_reason,
    }
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

/// Run one turn with a JSON array of session attachment IDs. Existing text-only
/// callers should continue using `chorus_run_turn`.
///
/// # Safety
/// `e` must be live; `agent`, `prompt`, and `attachment_ids_json` must be
/// valid NUL-terminated UTF-8 strings. The IDs argument must encode an array.
#[no_mangle]
pub unsafe extern "C" fn chorus_run_turn_with_attachments(
    e: *mut ChorusEngine,
    agent: *const c_char,
    prompt: *const c_char,
    attachment_ids_json: *const c_char,
    allow: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agent), Some(prompt)) = (borrow(agent), borrow(prompt)) else {
        return err("agent/prompt must be valid UTF-8");
    };
    let attachment_ids = match borrow_attachment_ids(attachment_ids_json) {
        Ok(ids) => ids,
        Err(e) => return err(e),
    };
    let policy = Policy::from_level(allow);
    match catch_unwind(AssertUnwindSafe(|| {
        engine
            .session
            .run_turn_with_attachments(agent, "openai", prompt, policy, &attachment_ids)
    })) {
        Ok(Ok(t)) => out(serde_json::to_string(&turn_json(t)).unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in run_turn_with_attachments"),
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

/// Interactive turn with a JSON array of session attachment IDs.
///
/// # Safety
/// `e` must be live; strings must be valid NUL-terminated UTF-8; `callback`
/// must be non-null and valid for the duration of this call; `ctx` is owned
/// by the caller.
#[no_mangle]
pub unsafe extern "C" fn chorus_run_turn_interactive_with_attachments(
    e: *mut ChorusEngine,
    agent: *const c_char,
    prompt: *const c_char,
    attachment_ids_json: *const c_char,
    callback: Option<PermissionCallback>,
    ctx: *mut std::ffi::c_void,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agent), Some(prompt)) = (borrow(agent), borrow(prompt)) else {
        return err("agent/prompt must be valid UTF-8");
    };
    let attachment_ids = match borrow_attachment_ids(attachment_ids_json) {
        Ok(ids) => ids,
        Err(e) => return err(e),
    };
    let Some(callback) = callback else {
        return err("null permission callback");
    };
    let ctx_usize = ctx as usize;
    let mut decide = |ask: &chorus_cli::session::PermissionAsk| {
        let payload = serde_json::json!({
            "tool": ask.tool, "kind": ask.kind, "detail": ask.detail
        })
        .to_string();
        let c = CString::new(payload).unwrap_or_default();
        if callback(ctx_usize as *mut std::ffi::c_void, c.as_ptr()) != 0 {
            chorus_cli::session::Permission::Allow
        } else {
            chorus_cli::session::Permission::Deny
        }
    };
    match catch_unwind(AssertUnwindSafe(|| {
        engine.session.run_turn_decide_with_attachments(
            agent,
            "openai",
            prompt,
            &attachment_ids,
            &mut decide,
        )
    })) {
        Ok(Ok(t)) => out(serde_json::to_string(&turn_json(t)).unwrap()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in run_turn_interactive_with_attachments"),
    }
}

// ---- transcript --------------------------------------------------

#[derive(Serialize)]
struct EventJson {
    seq: i64,
    kind: &'static str,
    actor: String,
    body: String,
    model_id: Option<String>,
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
                .map(|(seq, kind, actor, body, model_id)| EventJson {
                    seq,
                    kind,
                    actor,
                    body,
                    model_id,
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
            } else if name.contains("gemini") || name.contains("antigravity") || name.contains("agy") {
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
    if specs.is_empty() {
        return err("relay needs at least one agent");
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

/// Run a relay chain with session attachment IDs encoded as a JSON array.
///
/// # Safety
/// `e` must be live; `agents`, `prompt`, and `attachment_ids_json` must be
/// valid NUL-terminated UTF-8 strings; IDs must be a JSON integer array.
#[no_mangle]
pub unsafe extern "C" fn chorus_run_relay_with_attachments(
    e: *mut ChorusEngine,
    agents: *const c_char,
    prompt: *const c_char,
    attachment_ids_json: *const c_char,
    allow: c_int,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else {
        return err("null engine");
    };
    let (Some(agents), Some(prompt)) = (borrow(agents), borrow(prompt)) else {
        return err("agents/prompt must be valid UTF-8");
    };
    let attachment_ids = match borrow_attachment_ids(attachment_ids_json) {
        Ok(ids) => ids,
        Err(e) => return err(e),
    };
    let specs = specs_from_csv(agents);
    if specs.is_empty() {
        return err("relay needs at least one agent");
    }
    let policy = Policy::from_level(allow);
    match catch_unwind(AssertUnwindSafe(|| {
        engine
            .session
            .run_relay_with_attachments(&specs, prompt, policy, &attachment_ids)
    })) {
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
        Err(_) => err("panic in run_relay_with_attachments"),
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

/// Kill the running agent and its process tree. For app quit.
#[no_mangle]
pub extern "C" fn chorus_shutdown() {
    chorus_cli::shutdown();
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
/// (`.claude/skills`, `.agents/skills`), and the user-global roots — `agents`
/// (`~/.agents/skills`), `user` (`~/.claude/skills`), `codex`, `gemini`.
/// See [`chorus_core::skills::visible_roots`]. Read-only.
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
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        chorus_core::skills::list_visible(std::path::Path::new(cwd), home.as_deref())
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name, "description": s.description,
                    "path": s.dir.join("SKILL.md").to_string_lossy(), "source": s.source,
                })
            })
            .collect::<Vec<_>>()
    }));
    match r {
        Ok(skills) => out(serde_json::json!({ "ok": true, "skills": skills }).to_string()),
        Err(_) => err("panic in list_skills"),
    }
}

/// Every account of every agent with what is known about its usage: Codex's
/// own 5-hour/weekly percentages, and for Claude the last limit it hit.
/// JSON: {"ok":true,"accounts":[{name,agent,account,dir,ready,login,rate_limits,hit}]}
#[no_mangle]
pub extern "C" fn chorus_usage_report() -> *mut c_char {
    match catch_unwind(chorus_cli::limits::usage_report) {
        Ok(rows) => out(serde_json::json!({ "ok": true, "accounts": rows }).to_string()),
        Err(_) => err("panic in usage_report"),
    }
}

/// Add an extra account (`name` = a-z0-9-_) for `agent`. Creates its config
/// dir; the user then logs in with the returned command.
/// JSON: {"ok":true,"dir":"…","login":"…"}
///
/// # Safety
/// `agent` and `name` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_add_account(
    agent: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(agent), Some(name)) = (borrow(agent), borrow(name)) else {
        return err("agent/name must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| chorus_cli::accounts::add(agent, name))) {
        Ok(Ok(dir)) => out(serde_json::json!({
            "ok": true,
            "dir": dir.to_string_lossy(),
            "login": chorus_cli::accounts::login_command(agent, Some(name)),
        })
        .to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in add_account"),
    }
}

/// Remove an extra account and its config dir (its login). JSON: {"ok":true}
///
/// # Safety
/// `agent` and `name` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn chorus_remove_account(
    agent: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(agent), Some(name)) = (borrow(agent), borrow(name)) else {
        return err("agent/name must be valid UTF-8");
    };
    match catch_unwind(AssertUnwindSafe(|| {
        chorus_cli::accounts::remove(agent, name)
    })) {
        Ok(Ok(())) => out(serde_json::json!({ "ok": true }).to_string()),
        Ok(Err(e)) => err(format!("{e:#}")),
        Err(_) => err("panic in remove_account"),
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

/// Build file tree for workspace with git statuses and agent focus marker.
#[no_mangle]
pub unsafe extern "C" fn chorus_file_tree_json(
    cwd: *const c_char,
    focus_file: *const c_char,
    max_depth: c_int,
) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let focus = borrow(focus_file);
    let depth = if max_depth <= 0 { 6 } else { max_depth as usize };

    let r = catch_unwind(AssertUnwindSafe(|| {
        let tree = chorus_core::file_tree::build_file_tree(Path::new(cwd), focus, depth);
        serde_json::to_string(&serde_json::json!({
            "ok": true,
            "tree": tree
        })).unwrap()
    }));

    match r {
        Ok(s) => out(s),
        Err(_) => err("panic in file_tree_json"),
    }
}

/// Compute Myers aligned diff with spacer lines for side-by-side or unified view.
#[no_mangle]
pub unsafe extern "C" fn chorus_aligned_diff_json(
    old_text: *const c_char,
    new_text: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    let old_str = borrow(old_text).unwrap_or("");
    let new_str = borrow(new_text).unwrap_or("");
    let p = borrow(path).unwrap_or("file");

    let r = catch_unwind(AssertUnwindSafe(|| {
        let diff = chorus_core::diff_align::align_file_diff(p, old_str, new_str);
        serde_json::to_string(&serde_json::json!({
            "ok": true,
            "diff": diff
        })).unwrap()
    }));

    match r {
        Ok(s) => out(s),
        Err(_) => err("panic in aligned_diff_json"),
    }
}

/// Add an inline review comment.
#[no_mangle]
pub unsafe extern "C" fn chorus_add_review_comment(
    e: *mut ChorusEngine,
    path: *const c_char,
    line: c_int,
    comment: *const c_char,
    snippet: *const c_char,
) -> *mut c_char {
    let Some(engine) = e.as_mut() else { return err("null engine"); };
    let Some(path) = borrow(path) else { return err("path must be valid UTF-8"); };
    let Some(comment) = borrow(comment) else { return err("comment must be valid UTF-8"); };
    let snippet = borrow(snippet);

    let session_id = engine.session.session_id();
    let conn = engine.session.store().conn();

    match chorus_core::review::add_review_comment(
        conn,
        session_id,
        path,
        line as usize,
        comment,
        snippet,
    ) {
        Ok(id) => out(serde_json::json!({"ok": true, "id": id}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// List pending inline review comments for the current session.
#[no_mangle]
pub unsafe extern "C" fn chorus_pending_review_comments(e: *mut ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_mut() else { return err("null engine"); };
    let session_id = engine.session.session_id();
    let conn = engine.session.store().conn();

    match chorus_core::review::list_pending_comments(conn, session_id) {
        Ok(comments) => out(serde_json::json!({"ok": true, "comments": comments}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Clear pending review comments for the current session.
#[no_mangle]
pub unsafe extern "C" fn chorus_clear_review_comments(e: *mut ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_mut() else { return err("null engine"); };
    let session_id = engine.session.session_id();
    let conn = engine.session.store().conn();

    match chorus_core::review::clear_pending_comments(conn, session_id) {
        Ok(_) => out(serde_json::json!({"ok": true}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Generate agent steering prompt from pending review comments.
#[no_mangle]
pub unsafe extern "C" fn chorus_review_steering_prompt(e: *mut ChorusEngine) -> *mut c_char {
    let Some(engine) = e.as_mut() else { return err("null engine"); };
    let session_id = engine.session.session_id();
    let conn = engine.session.store().conn();

    match chorus_core::review::list_pending_comments(conn, session_id) {
        Ok(comments) => {
            let prompt = chorus_core::review::generate_steering_prompt(&comments);
            out(serde_json::json!({"ok": true, "prompt": prompt}).to_string())
        }
        Err(e) => err(format!("{e:#}")),
    }
}

/// Provision an isolated git worktree for an agent.
#[no_mangle]
pub unsafe extern "C" fn chorus_create_worktree(
    cwd: *const c_char,
    role: *const c_char,
) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let role = borrow(role).unwrap_or("agent");

    match chorus_core::worktrees::create_worktree(Path::new(cwd), role) {
        Ok(info) => out(serde_json::json!({"ok": true, "worktree": info}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// List active git worktrees.
#[no_mangle]
pub unsafe extern "C" fn chorus_list_worktrees(cwd: *const c_char) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };

    match chorus_core::worktrees::list_active_worktrees(Path::new(cwd)) {
        Ok(worktrees) => out(serde_json::json!({"ok": true, "worktrees": worktrees}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Merge a worktree branch into current workspace.
#[no_mangle]
pub unsafe extern "C" fn chorus_merge_worktree(
    cwd: *const c_char,
    branch: *const c_char,
) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let Some(branch) = borrow(branch) else { return err("branch must be valid UTF-8"); };

    match chorus_core::worktrees::merge_worktree(Path::new(cwd), branch) {
        Ok(outcome) => out(serde_json::json!({"ok": true, "outcome": outcome}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Discard and delete an active worktree.
#[no_mangle]
pub unsafe extern "C" fn chorus_remove_worktree(
    cwd: *const c_char,
    branch: *const c_char,
) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let Some(branch) = borrow(branch) else { return err("branch must be valid UTF-8"); };

    match chorus_core::worktrees::discard_worktree_by_branch(Path::new(cwd), branch) {
        Ok(()) => out(serde_json::json!({"ok": true}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// The total monetary cost of a session in USD.
#[no_mangle]
pub unsafe extern "C" fn chorus_session_cost_usd(
    db_path: *const c_char,
    session_id: i64,
) -> f64 {
    let Some(db) = borrow(db_path) else { return 0.0; };
    let Ok(store) = chorus_core::Store::open(Path::new(db)) else { return 0.0; };
    chorus_core::ledger::session_cost_usd(store.conn(), session_id).unwrap_or(0.0)
}

fn open_router_store(db_path: Option<&str>) -> std::result::Result<chorus_core::Store, String> {
    let resolved = if let Some(p) = db_path.filter(|p| !p.trim().is_empty()) {
        let path = std::path::PathBuf::from(p);
        if path.is_file() || path.extension().is_some_and(|ext| ext == "db") {
            path
        } else {
            path.join(".chorus/store.db")
        }
    } else {
        std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".chorus/store.db"))
            .ok_or_else(|| "HOME environment variable not set".to_string())?
    };

    if let Some(parent) = resolved.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    chorus_core::Store::open(&resolved).map_err(|e| format!("{e:#}"))
}

/// Instant in-process fast routing decision (<5ms) with learned memory lookup.
#[no_mangle]
pub unsafe extern "C" fn chorus_route_fast_json(prompt: *const c_char) -> *mut c_char {
    let p = borrow(prompt).unwrap_or("");
    let store = open_router_store(None).ok();
    let conn = store.as_ref().map(|s| s.conn());
    let decision = chorus_core::local_router::route_task_fast_with_db(conn, p, &[]);
    out(serde_json::json!({"ok": true, "decision": decision}).to_string())
}

/// List routes learned by the router flywheel.
#[no_mangle]
pub unsafe extern "C" fn chorus_router_list_learned_json(db_path: *const c_char) -> *mut c_char {
    let path = borrow(db_path);
    let store = match open_router_store(path) {
        Ok(s) => s,
        Err(e) => return err(e),
    };

    match chorus_core::local_router::list_learned_routes(store.conn(), 50) {
        Ok(routes) => out(serde_json::json!({ "ok": true, "routes": routes }).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Record a route decision / user choice into the learning flywheel.
#[no_mangle]
pub unsafe extern "C" fn chorus_router_record_learned_json(
    db_path: *const c_char,
    prompt: *const c_char,
    model: *const c_char,
    agent: *const c_char,
    reason: *const c_char,
) -> *mut c_char {
    let path = borrow(db_path);
    let p = borrow(prompt).unwrap_or("");
    let m = borrow(model).unwrap_or("");
    let a = borrow(agent).unwrap_or("");
    let r = borrow(reason).unwrap_or("");

    let store = match open_router_store(path) {
        Ok(s) => s,
        Err(e) => return err(e),
    };

    match chorus_core::local_router::record_route_learning(store.conn(), p, m, a, r) {
        Ok(item) => out(serde_json::json!({ "ok": true, "route": item }).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Clear all learned route history.
#[no_mangle]
pub unsafe extern "C" fn chorus_router_clear_learned_json(db_path: *const c_char) -> *mut c_char {
    let path = borrow(db_path);
    let store = match open_router_store(path) {
        Ok(s) => s,
        Err(e) => return err(e),
    };

    match chorus_core::local_router::clear_learned_routes(store.conn()) {
        Ok(()) => out(serde_json::json!({ "ok": true }).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Search sessions using SQLite FTS5 / LIKE search.
#[no_mangle]
pub unsafe extern "C" fn chorus_search_sessions_json(
    db_path: *const c_char,
    query: *const c_char,
) -> *mut c_char {
    let Some(db_path) = borrow(db_path) else { return err("db_path must be valid UTF-8"); };
    let q = borrow(query).unwrap_or("");

    let store = match chorus_core::Store::open(Path::new(db_path)) {
        Ok(s) => s,
        Err(e) => return err(format!("{e:#}")),
    };

    match chorus_core::search::search_sessions(store.conn(), q, 30) {
        Ok(hits) => out(serde_json::json!({"ok": true, "hits": hits}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// Export session to Markdown.
#[no_mangle]
pub unsafe extern "C" fn chorus_export_session_md(
    db_path: *const c_char,
    session_id: i64,
) -> *mut c_char {
    let Some(db_path) = borrow(db_path) else { return err("db_path must be valid UTF-8"); };

    let store = match chorus_core::Store::open(Path::new(db_path)) {
        Ok(s) => s,
        Err(e) => return err(format!("{e:#}")),
    };

    match chorus_core::search::export_session_markdown(store.conn(), session_id) {
        Ok(md) => out(serde_json::json!({"ok": true, "markdown": md}).to_string()),
        Err(e) => err(format!("{e:#}")),
    }
}

/// List available agent roles (built-in + .chorus/agents/*.md).
#[no_mangle]
pub unsafe extern "C" fn chorus_list_roles(cwd: *const c_char) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let roles = chorus_core::roles::load_roles(Path::new(cwd));
    out(serde_json::json!({"ok": true, "roles": roles}).to_string())
}

/// Save an agent role definition to .chorus/agents/<name>.md.
#[no_mangle]
pub unsafe extern "C" fn chorus_save_role(
    cwd: *const c_char,
    role_json: *const c_char,
) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let Some(rj) = borrow(role_json) else { return err("role_json must be valid UTF-8"); };

    let role: chorus_core::roles::AgentRole = match serde_json::from_str(rj) {
        Ok(r) => r,
        Err(e) => return err(format!("invalid role JSON: {e}")),
    };

    match role.save_to_dir(Path::new(cwd)) {
        Ok(path) => out(serde_json::json!({
            "ok": true,
            "path": path.to_string_lossy(),
            "name": role.name
        }).to_string()),
        Err(e) => err(format!("failed to save role: {e}")),
    }
}

/// Delete a custom agent role from .chorus/agents/<name>.md.
#[no_mangle]
pub unsafe extern "C" fn chorus_delete_role(
    cwd: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let Some(cwd) = borrow(cwd) else { return err("cwd must be valid UTF-8"); };
    let Some(name) = borrow(name) else { return err("name must be valid UTF-8"); };

    let file_path = Path::new(cwd)
        .join(chorus_core::roles::AGENTS_DIR)
        .join(format!("{name}.md"));

    if file_path.exists() {
        if let Err(e) = std::fs::remove_file(&file_path) {
            return err(format!("failed to delete role file: {e}"));
        }
    }
    out(serde_json::json!({"ok": true}).to_string())
}

/// Synthesize an agent role via AI heuristic / generator.
#[no_mangle]
pub unsafe extern "C" fn chorus_synthesize_role(
    task: *const c_char,
    write_mode: bool,
    name_hint: *const c_char,
) -> *mut c_char {
    let Some(task) = borrow(task) else { return err("task must be valid UTF-8"); };
    let hint = borrow(name_hint).filter(|s| !s.is_empty());

    let role = chorus_core::roles::synthesize_role(task, write_mode, hint);
    out(serde_json::json!({"ok": true, "role": role}).to_string())
}

/// List all turn checkpoints for a session.
#[no_mangle]
pub unsafe extern "C" fn chorus_checkpoint_list_json(
    db: *const c_char,
    session_id: i64,
) -> *mut c_char {
    let Some(db_path) = borrow(db) else { return err("db must be valid UTF-8"); };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let store = chorus_core::Store::open(Path::new(db_path)).map_err(|e| e.to_string())?;
        let records = chorus_core::checkpoint_dag::CheckpointDag::list_turn_checkpoints(&store, session_id)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(records).map_err(|e| e.to_string())?)
    }));

    match result {
        Ok(Ok(val)) => out(serde_json::json!({"ok": true, "checkpoints": val}).to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in checkpoint list"),
    }
}

/// Time-travel rewind across code and/or ledger history.
#[no_mangle]
pub unsafe extern "C" fn chorus_checkpoint_rewind_json(
    db: *const c_char,
    ws: *const c_char,
    session_id: i64,
    target_seq: i64,
    mode: *const c_char,
) -> *mut c_char {
    let Some(db_path) = borrow(db) else { return err("db must be valid UTF-8"); };
    let Some(ws_path) = borrow(ws) else { return err("ws must be valid UTF-8"); };
    let mode_str = borrow(mode).unwrap_or("code_and_ledger");

    let rewind_mode = match mode_str {
        "code_only" => chorus_core::checkpoint_dag::RewindMode::CodeOnly,
        "ledger_only" => chorus_core::checkpoint_dag::RewindMode::LedgerOnly,
        _ => chorus_core::checkpoint_dag::RewindMode::CodeAndLedger,
    };

    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let store = chorus_core::Store::open(Path::new(db_path)).map_err(|e| e.to_string())?;
        let ckpt = chorus_core::checkpoint::Checkpoint::open(Path::new(ws_path)).map_err(|e| e.to_string())?;
        let summary = chorus_core::checkpoint_dag::CheckpointDag::rewind_to_seq(
            &store,
            &ckpt,
            session_id,
            target_seq,
            rewind_mode,
        ).map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(summary).map_err(|e| e.to_string())?)
    }));

    match result {
        Ok(Ok(val)) => out(serde_json::json!({"ok": true, "summary": val}).to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in checkpoint rewind"),
    }
}

/// Generate a high-density Repo Map using Personalized PageRank in Rust.
#[no_mangle]
pub unsafe extern "C" fn chorus_repo_map_generate_json(
    ws: *const c_char,
    max_tokens: u32,
) -> *mut c_char {
    let Some(ws_path) = borrow(ws) else { return err("ws must be valid UTF-8"); };
    let path = Path::new(ws_path);

    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        // Collect candidate source files
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                    if matches!(ext, "rs" | "swift" | "ts" | "tsx" | "js" | "py" | "go") {
                        files.push(p);
                    }
                }
            }
        }

        let graph = chorus_core::repo_map::RepoMapEngine::build_graph(path, &files).map_err(|e| e.to_string())?;
        let ranks = chorus_core::repo_map::RepoMapEngine::compute_ppr(&graph, &[], 0.85, 25);
        let map_str = chorus_core::repo_map::RepoMapEngine::render_repo_map(&graph, &ranks, path, max_tokens as usize);

        Ok(serde_json::json!({
            "ok": true,
            "symbols_count": graph.definitions.len(),
            "repo_map": map_str
        }))
    }));

    match result {
        Ok(Ok(val)) => out(val.to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in repo map generation"),
    }
}

/// Verify workspace via fast linter/compiler checks.
#[no_mangle]
pub unsafe extern "C" fn chorus_verify_workspace_json(ws: *const c_char) -> *mut c_char {
    let Some(ws_path) = borrow(ws) else { return err("ws must be valid UTF-8"); };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let res = chorus_core::verify_loop::VerifyLoop::verify_workspace(Path::new(ws_path)).map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
    }));

    match result {
        Ok(Ok(val)) => out(serde_json::json!({"ok": true, "verification": val}).to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in verify workspace"),
    }
}

/// Structural AST diff analysis to evaluate semantic vs cosmetic line changes.
#[no_mangle]
pub unsafe extern "C" fn chorus_structural_diff_json(
    old_code: *const c_char,
    new_code: *const c_char,
) -> *mut c_char {
    let old_s = borrow(old_code).unwrap_or("");
    let new_s = borrow(new_code).unwrap_or("");

    let analysis = chorus_core::structural_diff::StructuralDiff::analyze(old_s, new_s);
    out(serde_json::json!({"ok": true, "analysis": analysis}).to_string())
}

static LOCAL_PAIRING: std::sync::OnceLock<chorus_daemon::pairing::PairingManager> = std::sync::OnceLock::new();

fn get_local_pairing() -> &'static chorus_daemon::pairing::PairingManager {
    LOCAL_PAIRING.get_or_init(|| {
        let endpoint = format!("http://{}", chorus_daemon::config::DEFAULT_HTTP_ADDR);
        chorus_daemon::pairing::PairingManager::new().with_endpoint(Some(endpoint))
    })
}

/// Initiate a pairing session. First queries the running daemon over the Unix domain socket;
/// falls back to local in-process pairing manager if daemon is offline.
#[no_mangle]
pub unsafe extern "C" fn chorus_pairing_initiate_json() -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let sock_path = chorus_daemon::config::default_chorus_dir()
            .join(chorus_daemon::config::DEFAULT_SOCKET_NAME);
        if sock_path.exists() {
            let client = chorus_daemon::client::DaemonClient::from_uds(&sock_path);
            if let Ok(session) = client.initiate_pairing() {
                return Ok(serde_json::json!({
                    "ok": true,
                    "session": session,
                }));
            }
        }
        let mgr = get_local_pairing();
        let session = mgr.initiate_host_session().map_err(|e: chorus_daemon::DaemonError| e.to_string())?;
        Ok(serde_json::json!({
            "ok": true,
            "session": session,
        }))
    }));
    match result {
        Ok(Ok(val)) => out(val.to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in pairing initiate"),
    }
}

/// List paired devices. First queries daemon over UDS, falls back to local in-process pairing manager.
#[no_mangle]
pub unsafe extern "C" fn chorus_pairing_devices_json() -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let sock_path = chorus_daemon::config::default_chorus_dir()
            .join(chorus_daemon::config::DEFAULT_SOCKET_NAME);
        if sock_path.exists() {
            let client = chorus_daemon::client::DaemonClient::from_uds(&sock_path);
            if let Ok(devices) = client.list_paired_devices() {
                return Ok(serde_json::json!({
                    "ok": true,
                    "devices": devices,
                }));
            }
        }
        let mgr = get_local_pairing();
        let devs: Vec<_> = mgr
            .list_devices()
            .iter()
            .map(|dev| {
                serde_json::json!({
                    "device_id": dev.device_id,
                    "device_name": dev.device_name,
                    "paired_at": dev.paired_at,
                    "last_seen_at": dev.last_seen_at,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "ok": true,
            "devices": devs,
        }))
    }));
    match result {
        Ok(Ok(val)) => out(val.to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in pairing devices"),
    }
}

/// Revoke a paired device by ID.
#[no_mangle]
pub unsafe extern "C" fn chorus_pairing_revoke_json(device_id: *const c_char) -> *mut c_char {
    let Some(id) = borrow(device_id) else {
        return err("device_id must be valid UTF-8");
    };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<serde_json::Value, String> {
        let sock_path = chorus_daemon::config::default_chorus_dir()
            .join(chorus_daemon::config::DEFAULT_SOCKET_NAME);
        if sock_path.exists() {
            let client = chorus_daemon::client::DaemonClient::from_uds(&sock_path);
            if let Ok(ok) = client.revoke_paired_device(id) {
                return Ok(serde_json::json!({
                    "ok": ok,
                }));
            }
        }
        let mgr = get_local_pairing();
        let ok = mgr.revoke_device(id);
        Ok(serde_json::json!({
            "ok": ok,
        }))
    }));
    match result {
        Ok(Ok(val)) => out(val.to_string()),
        Ok(Err(e)) => err(e),
        Err(_) => err("panic in pairing revoke"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn ffi_roles_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("chorus-ffi-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let cwd = CString::new(temp_dir.to_str().unwrap()).unwrap();

        // 1. Synthesize role
        let task = CString::new("Audit PostgreSQL migrations and index efficiency").unwrap();
        let hint = CString::new("db-optimizer").unwrap();
        let synth_ptr = unsafe { chorus_synthesize_role(task.as_ptr(), true, hint.as_ptr()) };
        let synth_str = unsafe { CStr::from_ptr(synth_ptr).to_str().unwrap() };
        let synth_json: serde_json::Value = serde_json::from_str(synth_str).unwrap();
        assert!(synth_json["ok"].as_bool().unwrap());
        let role = synth_json["role"].clone();
        assert_eq!(role["name"].as_str().unwrap(), "db-optimizer");
        unsafe { chorus_string_free(synth_ptr) };

        // 2. Save role
        let role_json = CString::new(role.to_string()).unwrap();
        let save_ptr = unsafe { chorus_save_role(cwd.as_ptr(), role_json.as_ptr()) };
        let save_str = unsafe { CStr::from_ptr(save_ptr).to_str().unwrap() };
        let save_json: serde_json::Value = serde_json::from_str(save_str).unwrap();
        assert!(save_json["ok"].as_bool().unwrap());
        unsafe { chorus_string_free(save_ptr) };

        // 3. List roles
        let list_ptr = unsafe { chorus_list_roles(cwd.as_ptr()) };
        let list_str = unsafe { CStr::from_ptr(list_ptr).to_str().unwrap() };
        let list_json: serde_json::Value = serde_json::from_str(list_str).unwrap();
        assert!(list_json["ok"].as_bool().unwrap());
        let roles = list_json["roles"].as_array().unwrap();
        assert!(roles.iter().any(|r| r["name"].as_str() == Some("db-optimizer")));
        unsafe { chorus_string_free(list_ptr) };

        // 4. Delete role
        let del_ptr = unsafe { chorus_delete_role(cwd.as_ptr(), hint.as_ptr()) };
        let del_str = unsafe { CStr::from_ptr(del_ptr).to_str().unwrap() };
        let del_json: serde_json::Value = serde_json::from_str(del_str).unwrap();
        assert!(del_json["ok"].as_bool().unwrap());
        unsafe { chorus_string_free(del_ptr) };

        // 5. Verify deleted from list
        let list_ptr2 = unsafe { chorus_list_roles(cwd.as_ptr()) };
        let list_str2 = unsafe { CStr::from_ptr(list_ptr2).to_str().unwrap() };
        let list_json2: serde_json::Value = serde_json::from_str(list_str2).unwrap();
        let roles2 = list_json2["roles"].as_array().unwrap();
        assert!(!roles2.iter().any(|r| r["name"].as_str() == Some("db-optimizer")));
        unsafe { chorus_string_free(list_ptr2) };
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn ffi_usage_report_antigravity() {
        let ptr = chorus_usage_report();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr).to_str().unwrap() };
        let json: serde_json::Value = serde_json::from_str(s).unwrap();
        assert!(json["ok"].as_bool().unwrap());
        let accounts = json["accounts"].as_array().unwrap();
        let anti = accounts.iter().find(|a| a["agent"] == "antigravity");
        assert!(anti.is_some());
        let anti_obj = anti.unwrap();
        println!("FFI Antigravity account JSON: {}", anti_obj);
        assert!(anti_obj["rate_limits"].is_object());
        assert!(anti_obj["rate_limits"]["secondary"].is_object());
        assert!(anti_obj["rate_limits"]["groups"].is_array());
        unsafe { chorus_string_free(ptr) };
    }

    #[test]
    fn ffi_router_learning_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("chorus-router-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("store.db");
        let db_c = CString::new(db_path.to_str().unwrap()).unwrap();
        let prompt_c = CString::new("เขียน SwiftUI Animated Transition").unwrap();
        let model_c = CString::new("claude-sonnet-4-6").unwrap();
        let agent_c = CString::new("claude").unwrap();
        let reason_c = CString::new("งานเกี่ยวกับ UI แนะนำ Sonnet").unwrap();

        // 1. Record
        let rec_ptr = unsafe {
            chorus_router_record_learned_json(
                db_c.as_ptr(),
                prompt_c.as_ptr(),
                model_c.as_ptr(),
                agent_c.as_ptr(),
                reason_c.as_ptr(),
            )
        };
        assert!(!rec_ptr.is_null());
        let rec_str = unsafe { CStr::from_ptr(rec_ptr).to_str().unwrap() };
        let rec_json: serde_json::Value = serde_json::from_str(rec_str).unwrap();
        assert!(rec_json["ok"].as_bool().unwrap());
        unsafe { chorus_string_free(rec_ptr) };

        // 2. List
        let list_ptr = unsafe { chorus_router_list_learned_json(db_c.as_ptr()) };
        assert!(!list_ptr.is_null());
        let list_str = unsafe { CStr::from_ptr(list_ptr).to_str().unwrap() };
        let list_json: serde_json::Value = serde_json::from_str(list_str).unwrap();
        assert!(list_json["ok"].as_bool().unwrap());
        let routes = list_json["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0]["chosen_model"].as_str().unwrap(), "claude-sonnet-4-6");
        unsafe { chorus_string_free(list_ptr) };

        // 3. Clear
        let clear_ptr = unsafe { chorus_router_clear_learned_json(db_c.as_ptr()) };
        assert!(!clear_ptr.is_null());
        let clear_str = unsafe { CStr::from_ptr(clear_ptr).to_str().unwrap() };
        let clear_json: serde_json::Value = serde_json::from_str(clear_str).unwrap();
        assert!(clear_json["ok"].as_bool().unwrap());
        unsafe { chorus_string_free(clear_ptr) };

        // 4. Verify list is empty
        let list_ptr2 = unsafe { chorus_router_list_learned_json(db_c.as_ptr()) };
        let list_str2 = unsafe { CStr::from_ptr(list_ptr2).to_str().unwrap() };
        let list_json2: serde_json::Value = serde_json::from_str(list_str2).unwrap();
        assert_eq!(list_json2["routes"].as_array().unwrap().len(), 0);
        unsafe { chorus_string_free(list_ptr2) };

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn ffi_pairing_lifecycle() {
        // 1. Initiate pairing
        let init_ptr = unsafe { chorus_pairing_initiate_json() };
        assert!(!init_ptr.is_null());
        let init_str = unsafe { CStr::from_ptr(init_ptr).to_str().unwrap() };
        let init_json: serde_json::Value = serde_json::from_str(init_str).unwrap();
        assert!(init_json["ok"].as_bool().unwrap(), "pairing initiate failed: {init_str}");
        let session = &init_json["session"];
        assert!(!session["pairing_token"].as_str().unwrap().is_empty());
        assert!(!session["endpoint_url"].as_str().unwrap().is_empty());
        unsafe { chorus_string_free(init_ptr) };

        // 2. List devices
        let dev_ptr = unsafe { chorus_pairing_devices_json() };
        assert!(!dev_ptr.is_null());
        let dev_str = unsafe { CStr::from_ptr(dev_ptr).to_str().unwrap() };
        let dev_json: serde_json::Value = serde_json::from_str(dev_str).unwrap();
        assert!(dev_json["ok"].as_bool().unwrap());
        unsafe { chorus_string_free(dev_ptr) };

        // 3. Revoke nonexistent device returns false
        let fake_id = CString::new("fake-device-123").unwrap();
        let rev_ptr = unsafe { chorus_pairing_revoke_json(fake_id.as_ptr()) };
        assert!(!rev_ptr.is_null());
        let rev_str = unsafe { CStr::from_ptr(rev_ptr).to_str().unwrap() };
        let rev_json: serde_json::Value = serde_json::from_str(rev_str).unwrap();
        assert!(!rev_json["ok"].as_bool().unwrap());
        unsafe { chorus_string_free(rev_ptr) };
    }
}


