/* Chorus core — C ABI for the native shells.
 *
 * Every returned `char *` is owned by the caller and must be passed to
 * `chorus_string_free`. Every such string is JSON: either the documented
 * success shape or {"ok":false,"error":"..."}.
 *
 * `ChorusEngine *` is opaque; free with `chorus_engine_free`.
 * All calls block — invoke them off the UI thread.
 */
#ifndef CHORUS_H
#define CHORUS_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ChorusEngine ChorusEngine;

/* Free a string returned by any chorus_* function. */
void chorus_string_free(char *p);

/* Open a store (creating if needed) and start a new session.
 * Returns NULL on failure. */
ChorusEngine *chorus_engine_open(const char *db_path,
                                 const char *cwd,
                                 const char *title);

/* Reattach to an existing session by id. Returns NULL on failure. */
ChorusEngine *chorus_engine_reopen(const char *db_path, int64_t session_id);

/* Free an engine handle. */
void chorus_engine_free(ChorusEngine *e);

/* The session's numeric id, or -1 if `e` is NULL. */
int64_t chorus_engine_session_id(const ChorusEngine *e);

/* Curated integration catalog. JSON list includes reviewed entries and
 * install/project state. Actions: install, update, remove, enable, disable. */
char *chorus_catalog_list_json(const char *project);
char *chorus_catalog_action_json(const char *project, const char *id,
                                 const char *action);

/* What an action would create or remove ({"ok":true,"plan":{items:[...]}}).
 * Touches nothing. Same arguments as chorus_catalog_action_json. */
char *chorus_catalog_plan_json(const char *project, const char *id,
                               const char *action);

/* Per-integration dashboard: {"ok":true,"dashboard":{...}}. Read-only. */
char *chorus_catalog_dashboard_json(const char *project, const char *id);

/* Run a "run" action: {"ok":true,"result":{"ok":bool,"output":"..."}}.
 * Confirm the action's effects with the user first. */
char *chorus_catalog_run_json(const char *project, const char *id, const char *key);

/* Phase of the running catalog action: {"ok":true,"progress":null|{id,current,done}}. */
char *chorus_catalog_progress_json(void);

/* Import original bytes and optional prepared text or normalized image.
 * Pass NULL for absent strings/data. Prepared text and normalized image are
 * mutually exclusive. JSON: {"ok":true,"attachment":{"id":N,"name":"...",
 * "media_type":"...","size":N,"status":"staged","readable":bool}}. */
char *chorus_import_attachment(ChorusEngine *e, const char *name,
                               const char *media_type,
                               const uint8_t *bytes, size_t bytes_len,
                               const char *derived_text,
                               const uint8_t *normalized_image,
                               size_t normalized_image_len,
                               const char *normalized_image_mime);

/* List attachment metadata for this session.
 * JSON: {"ok":true,"attachments":[{id,name,media_type,size,status,readable}]} */
char *chorus_list_attachments(const ChorusEngine *e);

/* Tombstone a session attachment. JSON: {"ok":true}. */
char *chorus_delete_attachment(const ChorusEngine *e, int64_t attachment_id);

/* Run one turn. `allow` != 0 approves permission requests.
 * JSON: {"ok":true,"user_seq":N,"agent_event_count":N,"message":"...",
 *        "context_used":N|null,"context_window":N|null,"stop_reason":"..."} */
char *chorus_run_turn(ChorusEngine *e,
                      const char *agent,
                      const char *prompt,
                      int allow);

/* Attachment-aware turn; attachment_ids_json is a JSON integer array (use
 * "[]" for no files). Same result shape as chorus_run_turn. */
char *chorus_run_turn_with_attachments(ChorusEngine *e, const char *agent,
                                       const char *prompt,
                                       const char *attachment_ids_json,
                                       int allow);

/* A permission callback. `ask_json` is {"tool":"...","kind":"...","detail":"..."};
 * return non-zero to allow. Called on the calling thread — block here while
 * you show a dialog. */
typedef int (*ChorusPermissionCallback)(void *ctx, const char *ask_json);

/* Run one turn, routing each permission request to `callback`.
 * Same JSON result shape as chorus_run_turn. */
char *chorus_run_turn_interactive(ChorusEngine *e, const char *agent,
                                  const char *prompt,
                                  ChorusPermissionCallback callback, void *ctx);

/* Interactive attachment-aware turn; IDs use the same JSON array format. */
char *chorus_run_turn_interactive_with_attachments(
    ChorusEngine *e, const char *agent, const char *prompt,
    const char *attachment_ids_json,
    ChorusPermissionCallback callback, void *ctx);

/* The whole session as a JSON array:
 * [{"seq":N,"kind":"msg","actor":"...","body":"...",
 *   "model_id":"..."}, ...]; model_id is null when unavailable. */
char *chorus_transcript_json(const ChorusEngine *e);

/* Only events with seq > `after` (0 = whole session). Same JSON shape.
 * A UI polls this with the highest seq it has rendered. */
char *chorus_events_since(const ChorusEngine *e, int64_t after);

/* Memory inspector data: approved facts + the pending proposal queue.
 * JSON: {"ok":true,"facts":["...",...],
 *        "pending":[{"id":N,"scope":N,"text":"..."},...]} */
char *chorus_memory_state(const ChorusEngine *e);

/* Approve (approve != 0) or reject a proposal by id. JSON: {"ok":true} */
char *chorus_memory_review(const ChorusEngine *e, int64_t proposal_id, int approve);

/* The workspace diff since the session's base checkpoint (agent edits).
 * JSON: {"ok":true,"diff":"..."} */
char *chorus_working_diff(const ChorusEngine *e);

/* Take a manual checkpoint. JSON: {"ok":true,"sha":"..."} */
/* Stop the running turn; callable from any thread. {"ok":true,"was_running":bool} */
char *chorus_cancel(void);

/* Kill the running agent and its whole process tree. Call from applicationWillTerminate. */
void chorus_shutdown(void);

/* System monitor: typed JSON request (op: snapshot|stop|wait_exit|disk|
 * preview|apply) → {"ok":true,"result":...}. `e` may be NULL; when given,
 * its checkpoint base is protected from checkpoint pruning. Blocking. */
char *chorus_monitor_json(const ChorusEngine *e, const char *request);

/* Skills visible in a workspace (chorus/project/user). {"ok":true,"skills":[{name,description,path,source}]} */
char *chorus_list_skills(const char *cwd);

/* Accounts & usage. Every account of every agent, with Codex's 5h/weekly
   percentages and Claude's last limit hit. Accounts are config dirs. */
char *chorus_usage_report(void);
char *chorus_add_account(const char *agent, const char *name);
char *chorus_remove_account(const char *agent, const char *name);

/* Rename / delete a session by store path (no open engine needed). {"ok":true} */
char *chorus_rename_session(const char *db_path, int64_t id, const char *title);
char *chorus_delete_session(const char *db_path, int64_t id);

/* Re-ask an agent for its models and refresh the cache (slow: opens a session). {"ok":true,"count":N} */
char *chorus_refresh_models(const char *agent_id, const char *cwd);

/* Share memory across a project: facts/approve queue/MCP go to `path`. {"ok":true} */
char *chorus_set_memory_db(ChorusEngine *e, const char *path);

/* Live signals from every running turn, one JSON push per call, on the
 * turn's thread: {"session_id":N,"seq":N,"kind":"...","text":"..."}.
 * kind = thought|message|tool|tool_done ("status\ttitle")|usage ("used\tsize")
 *      |events (ledger reaches seq: pull the delta)|status (running|idle|failed).
 * `seq` = newest ledger seq known for that session. String valid only during
 * the call. NULL cb clears. Signals only; a dropped push is recovered by the
 * next chorus_reader_events_since. */
typedef void (*chorus_push_cb)(void *ctx, const char *json);
void chorus_set_push_callback(chorus_push_cb cb, void *ctx);

/* Read-only second handle onto one session (SQLite WAL reads while a turn
 * writes). Never touches the engine handle's serial queue (CLAUDE.md 7.7).
 * Open on the engine queue right after opening `e`; then use it on its own
 * thread. NULL on failure. */
typedef struct ChorusReader ChorusReader;
ChorusReader *chorus_reader_open(const ChorusEngine *e);
void chorus_reader_free(ChorusReader *r);
/* Same JSON array as chorus_events_since. */
char *chorus_reader_events_since(const ChorusReader *r, int64_t after);
/* Same JSON as chorus_session_usage. */
char *chorus_reader_usage(const ChorusReader *r);
/* Same JSON as chorus_working_diff, staged in a private index. */
char *chorus_reader_working_diff(const ChorusReader *r);

/* Mark reviewed: snapshot shadow repo as new baseline. JSON: {"ok":true,"base":"..."} */
char *chorus_mark_reviewed(ChorusEngine *e);

/* Check if user committed between turns; advances baseline. JSON: {"ok":true,"moved":bool} */
char *chorus_check_user_commit(ChorusEngine *e);

/* Scoped working diff: scope = session|latest_turn|current_agent, include_projected (0/1). */
char *chorus_working_diff_scoped(const ChorusEngine *e, const char *scope, int include_projected);
char *chorus_reader_working_diff_scoped(const ChorusReader *r, const char *scope, int include_projected);

/* File diff stats report: fast numstat path. JSON: {"ok":true,"report":{...}} */
char *chorus_reader_diff_stats(const ChorusReader *r, const char *scope, int include_projected);

/* Single file diff. JSON: {"ok":true,"diff":"..."} */
char *chorus_reader_file_diff(const ChorusReader *r, const char *scope, const char *path);

char *chorus_snapshot(const ChorusEngine *e, const char *message);

/* Roll the workspace back to `sha` (NULL/"" = the session base).
 * Destructive — confirm with the user first. JSON: {"ok":true} */
char *chorus_restore(const ChorusEngine *e, const char *sha);

/* Reject a hunk selection: reverse-apply `patch` (containing only the
 * rejected hunks) to the workspace. JSON: {"ok":true} */
char *chorus_reject_hunks(const ChorusEngine *e, const char *patch);

/* Run one turn against a relay chain. `agents` is comma-separated names
 * (e.g. "codex,claude-opus,gemini"). On a rate-limit / quota / context
 * failure the chain hands off to the next link automatically.
 * JSON: {"ok":true,"answered_by":"...","message":"...",
 *        "context_used":N|null,"context_window":N|null,"stop_reason":"...",
 *        "handoffs":[["from","to","why"], ...]} */
char *chorus_run_relay(ChorusEngine *e, const char *agents,
                       const char *prompt, int allow);

/* Attachment-aware relay chain. attachment_ids_json is a JSON integer array. */
char *chorus_run_relay_with_attachments(ChorusEngine *e, const char *agents,
                                        const char *prompt,
                                        const char *attachment_ids_json,
                                        int allow);

/* Ask the local-only router to propose among supplied candidate tuples.
 * task_json and candidates_json must match chorus_core::router TaskSummary and
 * CandidateSet JSON. preset is "economy" | "balanced" | "max"; NULL or an
 * unknown name means balanced. Candidates without "tier" get one from the
 * curated table (CHORUS_MODEL_TIERS overrides). JSON success:
 * {"ok":true,"route":{"status":..., "result":...}}; routing unavailability
 * is returned in route, not error. */
char *chorus_route_json(const char *task_json, const char *candidates_json,
                        const char *model, const char *preset);

/* The session's route preset, or "preset":null when not set this run (apply
 * the Settings default with chorus_set_route_preset). Lists every preset for
 * the composer menu.
 * JSON: {"ok":true,"preset":"balanced"|null,"label":"สมดุล"|null,
 *        "presets":[{"id":"economy","label":"ประหยัด","hint":"..."}, ...]} */
char *chorus_route_preset(const ChorusEngine *e);

/* Set the session's route preset ("economy" | "balanced" | "max"). Unknown
 * names are an error. Kept across engine reopen for this process only.
 * JSON: same shape as chorus_route_preset. */
char *chorus_set_route_preset(ChorusEngine *e, const char *preset);

/* Local Ollama router setup. Inventory success:
 * {"ok":true,"models":[{"name":"...","digest":...,"size_bytes":...}]}.
 * Pull success: {"ok":true}; failures use the standard error shape.
 * Pull may block for up to two hours; call off the UI thread. */
char *chorus_router_models_json(const char *model);
char *chorus_router_pull_json(const char *model);

/* 1-Click In-Process Model Storage & Lifecycle Management.
 * JSON: {"ok":true,"models":[{"status":"not_installed"|"ready"|"corrupted",...}]} */
char *chorus_model_manager_list_json(void);

/* Atomically verify and install a downloaded temporary file into
 * ~/Library/Application Support/Chorus/models/
 * JSON: {"ok":true,"path":"..."} */
char *chorus_model_manager_finalize_json(const char *temp_path, const char *model_id);

/* Delete a local model file to reclaim disk space.
 * JSON: {"ok":true,"deleted":bool} */
char *chorus_model_manager_delete_json(const char *model_id);

/* Compare mode: fork the session per agent, run `prompt` on each.
 * JSON: {"ok":true,"branches":[{"agent":"...","session_id":N,
 *        "message":"...","stop_reason":"...","error":null}, ...]} */
char *chorus_compare(ChorusEngine *e, const char *agents,
                     const char *prompt, int allow);

/* Hand the session to `to`. `goal` required; `done_json`/`pending_json`
 * are JSON string arrays or NULL.
 * JSON: {"ok":true,"handoff_seq":N,"from":"...","to":"...",
 *        "brief_markdown":"...","brief_json":"...","echo_back":"...",
 *        "stop_reason":"..."} */
char *chorus_handoff(ChorusEngine *e,
                     const char *to,
                     const char *reason,
                     const char *goal,
                     const char *done_json,
                     const char *pending_json,
                     int allow);

#ifdef __cplusplus
}
#endif


/* Warm tier (summary rungs). `chorus_compact` may be slow — it can run a
 * local model per event — so call it off the UI thread and never inside a
 * turn. Context assembly reads these rows and never generates them. */
char *chorus_uncompacted(const ChorusEngine *e);
/* JSON: {"ok":true,"compacted":N,"remaining":N,"model":"...",
 *        "took_ms":N,"hit_budget":true|false}
 * A pass stops at whichever comes first, `limit` events or an internal wall
 * clock budget (CHORUS_COMPACT_BUDGET_MS, default 3s), and always reports
 * what is left — so calling it again after the next turn is the whole
 * catch-up strategy. `hit_budget` means slow summariser, not failure. */
char *chorus_compact(const ChorusEngine *e, int limit);

/* goal / done / pending / constraints for this session. */
char *chorus_task_state(const ChorusEngine *e);

/* Last stored usage for this session (seeds the context meter on reopen).
 * JSON: {"ok":true,"usage":null|{"context_used":N|null,"context_window":N|null,
 *        "tokens_in":N,"tokens_out":N,"cost_usd":X|null,"updated_ms":N}} */
char *chorus_session_usage(const ChorusEngine *e);

/* Exactly the context a turn would hand an agent right now, assembled from
 * the ledger. Not a description of it — the thing itself. */
char *chorus_assembled_context(const ChorusEngine *e, long long budget);

/* Every session in db_path, newest activity first. Does not need a
 * ChorusEngine — a store-level query, no shadow-git/MCP handles opened.
 * JSON: {"ok":true,"sessions":[{"id":N,"title":"...","cwd":"...",
 *        "started_ms":N,"last_ms":N,"events":N,"agents":["..."],
 *        "goal":"...","forked_from":N|null}, ...]} */
char *chorus_list_sessions(const char *db_path);

/* Every agent Chorus knows how to bridge to, with whether it can run right
 * now. Cheap and non-blocking: no login, no network, no credential file is
 * ever read — existence only.
 * JSON: {"ok":true,"agents":[{"id":"...","label":"...",
 *        "models":[{"id":"...","label":"...","is_default":true}, ...],
 *        "auth":"ok"|"needs_login"|"missing_cli","hint":"...",
 *        "detail":"..."}, ...]} */
char *chorus_list_agents(void);

/* Pin the model an agent runs, by the id it reports itself (models[].id from
 * chorus_list_agents). Empty model clears the pin. Applied when the next turn
 * opens its session — without this a model picker only changes a label. */
char *chorus_set_model(ChorusEngine *e, const char *agent, const char *model);

/* One agent's current runnability, by id ("claude" | "codex"). Same shape
 * as one entry of chorus_list_agents. */
char *chorus_auth_status(const char *agent_id);

/* One-click handoff: goal/done/pending/constraints all come from the
 * session's own task_state() — nothing but `to` and a permission `level`
 * (0 deny · 1 read-only · 2 allow, same mapping as chorus_run_turn's
 * `allow`) is asked of the caller.
 * JSON: {"ok":true,"to":"...","goal":"...","echo_back":"..."} */
char *chorus_handoff_auto(ChorusEngine *e, const char *to, int level);

/* File tree and workspace navigation */
char *chorus_file_tree_json(const char *cwd, const char *focus_file, int max_depth);

/* Myers diff with spacer line pre-alignment */
char *chorus_aligned_diff_json(const char *old_text, const char *new_text, const char *path);

/* Inline review comments and steering loop */
char *chorus_add_review_comment(ChorusEngine *e, const char *path, int line, const char *comment, const char *snippet);
char *chorus_pending_review_comments(ChorusEngine *e);
char *chorus_clear_review_comments(ChorusEngine *e);
char *chorus_review_steering_prompt(ChorusEngine *e);

/* Git worktree orchestration */
char *chorus_create_worktree(const char *cwd, const char *role);
char *chorus_list_worktrees(const char *cwd);
char *chorus_merge_worktree(const char *cwd, const char *branch);
char *chorus_remove_worktree(const char *cwd, const char *branch);

/* Monetary Cost Accounting */
double chorus_session_cost_usd(const char *db_path, int64_t session_id);

/* In-process fast routing and learning memory flywheel */
char *chorus_route_fast_json(const char *prompt, const char *preset);
char *chorus_router_list_learned_json(const char *db_path);
/* Record a user override under preset (NULL = balanced). Never record the
 * router's own pick. */
char *chorus_router_record_learned_json(const char *db_path, const char *prompt, const char *model, const char *agent, const char *reason, const char *preset);
char *chorus_router_clear_learned_json(const char *db_path);

/* Cross-session search and export */
char *chorus_search_sessions_json(const char *db_path, const char *query);
char *chorus_export_session_md(const char *db_path, int64_t session_id);

/* Agent roles and AI role synthesis */
char *chorus_list_roles(const char *cwd);
char *chorus_save_role(const char *cwd, const char *role_json);
char *chorus_delete_role(const char *cwd, const char *name);
char *chorus_synthesize_role(const char *task, bool write_mode, const char *name_hint);

/* Super App & Autonomous Harness APIs */
char *chorus_checkpoint_list_json(const char *db, int64_t session_id);
char *chorus_checkpoint_rewind_json(const char *db, const char *ws, int64_t session_id, int64_t target_seq, const char *mode);
char *chorus_repo_map_generate_json(const char *ws, uint32_t max_tokens);
char *chorus_verify_workspace_json(const char *ws);
/* "Trust this project": checks and hooks run only when true. {"ok","trusted"} */
char *chorus_project_trusted_json(const char *ws);
char *chorus_set_project_trusted(const char *ws, bool trusted);
char *chorus_structural_diff_json(const char *old_code, const char *new_code);

/* Syntax spans for the read-only code viewer. `path` picks the language by
 * extension. JSON: {"ok":true,"lines":[[[start,len,kind],...],...]} — one
 * entry per '\n'-separated line of `text`, offsets in UTF-8 bytes within the
 * line. kind: 0 keyword · 1 type · 2 string · 3 comment · 4 number ·
 * 5 function · 6 attribute/macro/key · 7 constant. */
char *chorus_highlight(const char *path, const char *text);

/* Composer (pure, no engine). Pasted HTML -> Markdown, capped; past a cap the
 * plain flavour is returned. {"ok","markdown","fallback":null|reason}.
 * Spans: {"ok","spans":[{"start","end","kind"}]} in UTF-16 units; kind is
 * heading1/2/3, strong, emphasis, strike, inline_code, code_block, link,
 * quote, list_item, marker. */
char *chorus_html_to_markdown(const char *html, const char *plain);
char *chorus_markdown_spans(const char *text);
/* media_types: JSON array. {"ok","note":null|"..."} — shown when attachments narrow the router. */
char *chorus_attachment_route_note(const char *media_types);

/* Remote Companion & Pairing Management (Phase 6) */
char *chorus_pairing_initiate_json(void);
char *chorus_pairing_devices_json(void);
char *chorus_pairing_revoke_json(const char *device_id);

#endif /* CHORUS_H */
