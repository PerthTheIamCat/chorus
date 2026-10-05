# Chorus attachments, integration catalog, and local router

Status: ready for implementation planning. Product decisions are recorded in the Wayfinder tickets; release gates below still require implementation verification. The local-only fallback is an explicit working assumption from the user's local-router direction and can be revised before implementation.

## Goal and scope

Give a macOS Chorus user three connected capabilities: attach images and files to a message and reuse them across agent handoffs; install curated agent integrations once and enable them per project; and have a small local router propose an available agent, model, and reasoning effort for each new user message. The user can always inspect and override the routing choice. Existing manual sends continue to work.

This is a product and implementation spec, not an implementation. Windows, direct Jev API calls, and generic arbitrary-source plugin installation are outside this release.

## Existing seams to preserve

- The composer currently owns only text. `Engine.send` passes one string over FFI; `session.rs` stores a text event and the ACP bridge sends a single text content block. Add attachment-aware entry points without breaking text-only callers. [Composer](../../apps/macos/Sources/Chorus/Composer.swift), [Engine](../../apps/macos/Sources/Chorus/Engine.swift), [session](../../crates/chorus-cli/src/session.rs), [ACP adapter](../../crates/chorus-acp/src/codex.rs).
- The ledger is append-only and its blob store deduplicates bytes. Its existing `event.blob` field stores oversized *message body text*; it is not a file relation. An attachment needs its own metadata, message relation, and deletion state. [Ledger](../../crates/chorus-core/src/ledger.rs), [blob store](../../crates/chorus-core/src/blob.rs).
- Project MCP entries are loaded from `.chorus/mcp.json` and passed to ACP `session/new`; project skills live at `.chorus/skills/`. Catalog state must compose with these user-authored sources without overwriting them. [MCP config](../../crates/chorus-core/src/mcp_config.rs), [skills](../../crates/chorus-core/src/skills.rs).
- The current model picker knows reported model IDs but derives some effort choices from suffixes. Routing must use live, ready agent/model/effort options and validate the final choice at dispatch. [Model picker](../../apps/macos/Sources/Chorus/ModelPicker.swift), [model pin](../../crates/chorus-acp/src/codex.rs).
- Core already has an Ollama call for compaction. Its free-form response and silent fallback are suitable for summarization but not for a routing decision. The router needs schema validation, bounded latency, and an explicit fallback result. [Compaction](../../crates/chorus-core/src/compaction.rs).

## 1. Attachments

### User contract

- The composer accepts file picker, drag and drop, and pasted images. Each selected item appears as a chip with name, type, size, preview when possible, and a remove action. A workspace `@file` mention remains a path reference and is visually distinct from a stored attachment.
- Copy the original bytes into a staged session attachment as soon as the user selects them, so moving or deleting the source file does not break the draft. On send, relate the attachment IDs and new user ledger event in one transaction. If preflight fails, keep the draft; if the agent fails after the event is committed, keep the visible message and offer retry. Remove unreferenced staged bytes when the user removes an unsent attachment.
- First-release agent-readable classes: images the macOS decoder can open and normalize to JPEG/PNG, UTF-8 or UTF-16 plain text/code, and PDFs whose text can be extracted locally. Animated content is not silently reduced to one frame. Keep the original bytes when creating a normalized image or extracted text. If an image cannot be decoded or a PDF has no extractable text, show a specific unsupported-content message rather than sending an empty substitute. Other files may be kept in the session but are marked unavailable to the agent until an adapter supports them.
- Limits: at most five attachments per message, 20 MB per file, and 50 MB combined. Validate before copying or sending. These are Chorus product limits; a selected bridge may impose a stricter limit discovered at runtime.
- Show the exact attachment names and any conversion (for example HEIC to JPEG or PDF to text) before sending. Never silently omit one attachment from the agent payload.
- If the user deletes an attachment after sending, immediately revoke further agent access and display a deleted marker in history. Remove stored bytes when no live attachment relation or existing event-body reference uses them. The ledger keeps the fact that an attachment existed, not its deleted contents.

### Delivery and handoff

- The selected agent receives the current message's supported images as ACP image blocks only when its live handshake advertises image support and the chosen model accepts images. Small text and extracted PDF content use an embedded resource where supported. Larger content is represented by its filename, size, page/section manifest, and a retrieval tool; any excerpt or truncation is marked. Do not infer support solely from an API provider's general documentation.
- Before dispatch, preflight every attachment against the selected agent/model and payload limits. If one is unsupported, keep the draft and show which file failed and why; the user may remove it or choose a capable target.
- A session attachment manifest gives a later agent names, types, originating message, and deletion state. The agent fetches older content only when needed; Chorus does not resend all old files automatically. The manifest must never expose deleted bytes.
- Agent-facing retrieval is session-scoped and returns only the requested attachment or bounded excerpt through Chorus's existing MCP channel. The tool process receives its session scope from Chorus, not from a model-supplied session ID. It validates attachment IDs, limits output, and logs access in the session. MCP tool results can carry text, image, and embedded resource content, but the active bridge's support still controls actual delivery. [MCP tool result spec](https://modelcontextprotocol.io/specification/2025-06-18/server/tools#tool-result).

### Data and module seams

- `AttachmentStore`: import original bytes; record name, detected media type, size, hash, owning session, and staged/sent/deleted status; relate attachment IDs to a ledger user event transactionally; list and tombstone attachments; garbage-collect unreachable bytes. Keep `event.blob` semantics unchanged and include both event-body and live attachment references in garbage-collection checks.
- `AttachmentDelivery`: accept attachment IDs plus the selected agent's live capabilities, and return ordered ACP content blocks or an explicit incompatibility error. It owns normalization and text extraction so Swift UI and each bridge do not implement different rules.
- Add a versioned attachment-aware FFI turn input while retaining the existing text-only entry point. Cross the FFI seam with IDs/metadata, not large byte arrays on every resend.
- If the message is retried or handed off, use the persisted ID and original bytes. If the attachment was deleted between selection and dispatch, stop with a specific error.

## 2. Curated integration catalog

### User contract

- A curated catalog lists source, maintainer, license, reviewed version, integration modes, required runtimes, executable actions, project/file access, network behavior, and any required secret *names*. Do not display secret values.
- Install an integration once at user scope; enable or disable it separately for each project. Show install health, version, update status, and why an integration is unavailable for a particular agent or project.
- First-release modes: MCP server, Chorus-projected skill, and callable CLI. The CLI binary must actually be available to enabled agent processes. Hook-based command rewriting and proxying model traffic are separate modes that require their own design; a CLI entry does not imply either effect.
- Initial curated entries: Graft via MCP and CLI, Serena via MCP, RTK via CLI and a scoped usage instruction, Headroom via MCP and CLI where supported. The catalog must describe the precise effect of each mode rather than claiming every product works as an MCP server.
- Updates are automatic **only for versions approved in the curated manifest shipped with a Chorus release**. On launch after a catalog change, download and verify the new version, activate for new sessions, run a health check, and roll back to the prior working version on failure. Do not replace binaries used by an active agent session. Show update history and allow disabling an integration immediately. This avoids an independent remote catalog update service in the first release.

### Installation and ownership

- Use an app-managed per-user tool directory and absolute executable paths; avoid global package installs and opaque vendor `init` scripts. The preview lists all files/config that Chorus will create or change. If a runtime prerequisite is missing, show that requirement and a recovery action instead of partly installing.
- Record catalog ID, version, source URL, digest, license, install method, runtime path, owned files, and health result. Only remove Chorus-owned files on uninstall; never delete another tool's global config or the user's project files.
- Project enablement has one source of truth in user-controlled Chorus settings keyed to the project, separate from the user-level install record. A checked-in project file cannot silently grant an integration permission; existing user-authored `.chorus/mcp.json` remains a distinct source. Compose enabled catalog MCP entries with that file at session creation; name collisions must be surfaced rather than silently overridden. Project skills remain in the current skill path. Add only enabled CLI paths to that agent process, not the user's global shell.
- An automatic update with the same declared permissions may activate after verification. If the new approved version requests broader filesystem/network/secret access or changes license terms, hold it for user review instead of expanding access silently.
- Disabling a project integration stops exposing it to *new* agent sessions immediately. An already running session keeps its launched process until it ends; the UI states this. Uninstall is unavailable while other projects still enable the integration, unless the user explicitly disables them too.
- Catalog metadata is trusted only when it comes from the maintained curated manifest; tool runtime output is still untrusted. Per-project enablement controls exposure to an agent, but it is not an operating-system sandbox. Do not claim that a network-enabled tool is confined to the project directory unless that is enforced.

### First-release install recipes

| Entry | Enabled effect | Excluded effect |
|---|---|---|
| Graft | Managed CLI plus MCP registration for a project | Running `graft init` or editing global agent hooks without review |
| Serena | Managed MCP launch with project-scoped configuration | Bundling or copying its source into Chorus |
| RTK | Managed CLI available to the agent plus scoped usage instruction | Transparent rewriting of every shell command |
| Headroom | Managed MCP or CLI mode declared in the entry | Transparent model-request proxy or wrapper mode |

## 3. Local pre-dispatch router

### User contract

- In Auto mode, each new user message gets an agent/model/reasoning-effort recommendation from the currently ready options. Show the selected target and a short task-based reason with a five-second countdown before dispatch. Send now and Cancel are visible; changing the choice, focusing the review controls, or invoking accessibility interaction pauses the countdown. A separate Manual mode pins the user's chosen target and bypasses the router; the existing picker remains the manual override surface.
- Use a small local model through Ollama for the first adapter. Discover locally installed models through Ollama's local inventory, then let the user select or install a supported router model during setup; show its download size and license before installation. Reject a cloud/remote model for the setting labeled local. Only the routing step is local. The selected work agent can still use its normal provider account. [Ollama model inventory](https://docs.ollama.com/api/tags).
- Route using the user's new task text, attachment type/size metadata, enabled integration capabilities, and the candidate list. Do not send attachment bytes, full ledger history, secrets, or project files to the selector. For a remote endpoint, require a separate explicit setting and disclosure; a default labeled local must connect to loopback.
- Require schema-constrained output containing one candidate ID from the live tuple list, a concise reason, and a confidence class. Keep agent/model/effort together in the candidate ID so the model cannot invent an unsupported combination. Ollama supports JSON-schema output; the application still validates it. [Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs).
- On timeout, malformed output, low confidence, or unavailable target, stop automatic dispatch and present the user's last valid manual selection for confirmation. If none exists, require a fresh selection. Do not silently call an account-backed router. A target that becomes unavailable during countdown also stops dispatch.

### Dispatch seam and decision record

- A `Router` module receives `TaskSummary` plus `CandidateSet` and returns `RouteProposal | RouteUnavailable`. `CandidateSet` is built from each agent's live model and effort metadata, media support, enabled integrations, and readiness. Deterministic validation removes impossible choices before and after the model call.
- Store the proposed choice, any user override, final dispatched target, router model/version, reason, and fallback reason as session event metadata. This supports explaining decisions and later quality review without storing the entire private router prompt.
- A route runs once per new user send, never recursively for internal handoffs or retries of identical inputs. Queued messages route when they reach dispatch, using the then-current candidate set. Changes to the draft or available candidates invalidate a pending proposal. Manual override applies to that send unless the user explicitly pins it. An explicit `@agent` directive retains its current meaning and bypasses automatic initial selection.
- The proposal chooses the first link in Chorus's existing account/provider failover chain. Preserve the user's failover settings, but preflight each later link against this turn's required media and integration capabilities. On a transient failure, use only a compatible ready link; if none remains, stop and explain rather than dropping an attachment or tool requirement. Do not invoke the router a second time for the handoff.
- Keep the local router process warm when practical, but bound startup/inference time and let the user cancel. Validate routing quality against a labeled set of real Chorus tasks and measure the *completed task* quality, latency, and account usage against manual selection before making local routing the default for everyone.

## Cross-feature interaction

- Attachment preflight can constrain router candidates: an image-bearing turn excludes models without verified image support. A model decision is invalidated if the user changes attachments before dispatch.
- A project-enabled integration may inform routing when a task specifically needs it. The router only receives integration capability names, never secrets or tool output.
- If a tool becomes unavailable after routing, report it at dispatch. Do not silently select a different model or change the user's override.

## Delivery order and acceptance

1. Add attachment data and composer import/preview/delete, then ACP capability-aware delivery and handoff access. Acceptance: a source file may move after import; later agents can still open it, and deleted attachments cannot be opened.
2. Add catalog manifest, managed install/update lifecycle, project enablement, and MCP/skill/CLI projections. Acceptance: enabling RTK in one project exposes its callable binary only in that project's agent process; disabling an MCP integration removes it from new sessions; failed update rolls back.
3. Add live candidate discovery, local router adapter, validation/fallback, review/override UI, and decision record. Acceptance: invalid router output never dispatches to an unavailable model, image tasks exclude non-image targets, and an absent Ollama server still allows a manual send.
4. Verify compatibility with the currently installed Codex, Claude, and Gemini ACP bridges on macOS. Their observed handshake, file support, and errors are release gates; the provider API documentation alone is not proof of bridge behavior.

## Primary references

- [ACP content blocks](https://agentclientprotocol.com/protocol/v1/content) and [prompt capabilities](https://agentclientprotocol.com/protocol/v1/prompt-capabilities)
- [MCP tool results and security considerations](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
- [Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs) and [macOS requirements](https://docs.ollama.com/macos)
- [Attachment bridge research](research/01-attachment-compatibility.md), [catalog research](research/02-integration-catalog.md), [router research](research/03-router-existing-agent.md)
