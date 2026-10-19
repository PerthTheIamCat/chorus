# Antigravity quota source (legit, no credential scraping)

Date: 2026-10-18. Status tags: [V] verified by running/fetching, [U] unverified.

## Answer
`agy -p "/usage" --output-format json` prints the quota as structured JSON. It is agy's own documented-in-CLI slash command, uses agy's own auth, runs 0 turns and 0 tokens. No ps, keychain or foreign-file access by Chorus.

## Evidence
1. [V] `agy --help` (local, ~/.local/bin/agy): `-p/--print` runs non-interactively; `--output-format text|json|stream-json`; `--disable-slash-commands` "Disable slash command and skill expansion in print mode" (so slash commands expand in print mode by default). No dedicated `usage`/`quota` subcommand (subcommands: agent, changelog, install, mcp, models, plugin, remote-control, update, ...).
2. [V] Command run locally: `agy -p "/usage" --output-format json` returned `"num_turns":0`, all usage tokens 0, and:
   `command.data.groups[].buckets[]` = `{id, name, window ("5h"|"weekly"), remaining_fraction (0..1), reset_time (RFC3339 UTC), description?}`.
   Groups seen: "Gemini Models" (gemini-weekly, gemini-5h), "Claude and GPT models" (3p-weekly, 3p-5h), "Gemini 4 Argon" (weekly only). Top-level `response` is also a TSV text rendering.
   Observed values: 3p-weekly 41.7% left, resets 2026-10-22T02:52:54Z; 5h buckets ~99-100%.
3. [V] Official plans page https://antigravity.google/docs/plans : Ultra refreshes every five hours; Pro 5-hour quota until weekly limit; others weekly. Lists "Model quotas (/usage)" as a CLI slash command and says usage is viewable in Settings. States no API for checking quota is described.
4. [V] No documented local status file/socket/API for quota found (plans page above; `agy remote-control status` only reports daemon state; changelog mentions AI credits in Remote Control settings UI only).
5. [V] ACP `usage_update` (https://agentclientprotocol.com/announcements/session-usage-stabilized.md): context token count, context window size, optional cumulative cost. No quota or rate-limit fields. [U] exact field names not in that page (schema is under /protocol/v1/prompt-turn#session-usage-updates; fetch of draft schema truncated).
6. [V] Chorus repo: crates/chorus-acp/src/lib.rs:182 and codex.rs:1148 consume `used`/`size`/`cost`; crates/chorus-core/src/usage.rs stores context/tokens/cost only. Chorus's antigravity_bridge.rs:395 itself emits usage_update with inputTokens/outputTokens (non-standard names). So ACP gives no 5h/weekly windows.
7. Third-party tools (WebSearch results, https://playbooks.com/skills/openclaw/skills/antigravity-balance and skainguyen1412/antigravity-usage): [V per search summary] the "antigravity-balance" skill extracts connection info from the language-server process arguments and queries a local GetUserStatus endpoint. That is exactly the ps/CSRF scraping removed in P3-1; REJECTED. antigravity-usage also offers multi-account Google OAuth; [U] not reviewed, and it would be Chorus holding a Google token, so not recommended. Could not find AIUsageBar's source: [U] its method unknown, do not copy.

## Ruled out
ps-arg token scraping, local language-server GetUserStatus replay, keychain/credential files, own OAuth client impersonating Antigravity. All violate CLAUDE.md §11 / 2026-10-17 P3-1.

## Recommendation
1. Poll `agy -p "/usage" --output-format json` (spawn with Chorus's normal sandbox/env; no extra args). Parse `command.data.groups[].buckets[]`. Map: window "5h" -> 5-hour, "weekly" -> weekly; bucket group by model family (Claude/GPT group vs Gemini group) to match the model Chorus is using. Cache ~60s; refresh after each Antigravity turn and on failure; short timeout (~10s); run off the turn queue's critical path.
2. Treat as pre-flight signal: remaining_fraction near 0 -> failover proactively; `reset_time` feeds the UI countdown.
3. Defensive: it's an interactive slash command exposed via print mode, so the JSON shape is not a contract [U]. Parse tolerantly, require `command.name=="usage"`, and fall back on any error/missing field.
4. Fallback: show "ไม่ทราบ" for windows, plus reactive detection from failure messages (HandoffReason::from_failure; changelog notes agy reports individual-quota exhaustion immediately).
5. Caveat [U]: `/usage` may require logged-in state; if agy is logged out it should error -> "ไม่ทราบ".
