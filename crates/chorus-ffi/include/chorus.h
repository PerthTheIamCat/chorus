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

/* Run one turn. `allow` != 0 approves permission requests.
 * JSON: {"ok":true,"user_seq":N,"agent_event_count":N,"message":"...",
 *        "context_used":N|null,"context_window":N|null,"stop_reason":"..."} */
char *chorus_run_turn(ChorusEngine *e,
                      const char *agent,
                      const char *prompt,
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

/* The whole session as a JSON array:
 * [{"seq":N,"kind":"msg","actor":"...","body":"..."}, ...] */
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
 * JSON: {"ok":true,"answered_by":"...","message":"...","stop_reason":"...",
 *        "handoffs":[["from","to","why"], ...]} */
char *chorus_run_relay(ChorusEngine *e, const char *agents,
                       const char *prompt, int allow);

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

#endif /* CHORUS_H */
