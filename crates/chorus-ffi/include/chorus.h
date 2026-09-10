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

/* The whole session as a JSON array:
 * [{"seq":N,"kind":"msg","actor":"...","body":"..."}, ...] */
char *chorus_transcript_json(const ChorusEngine *e);

/* Only events with seq > `after` (0 = whole session). Same JSON shape.
 * A UI polls this with the highest seq it has rendered. */
char *chorus_events_since(const ChorusEngine *e, int64_t after);

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
