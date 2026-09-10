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

#endif /* CHORUS_H */
