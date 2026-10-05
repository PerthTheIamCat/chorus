# Pre-dispatch router policy and UX

Type: grilling
Status: resolved
Blocked by: 03

## Question

Given the routing feasibility research, settle the router's first-release policy: whether a local model (for example, via Ollama) or an existing agent account performs routing, what task context it receives, how it chooses among available agent/model/reasoning combinations, how the UI presents rationale and override, and what happens on low confidence, failure, or an unavailable target. The agreed direction is automatic selection for every new task, with user override; direct Jev API calls are out of scope.

## Local option to evaluate

- Chorus already calls a local Ollama server for compaction in `crates/chorus-core/src/compaction.rs`; this path currently generates free-form text, so router output still needs a schema, validation against available targets, and a safe fallback.
- Ollama supports JSON-schema structured output through its local API: https://docs.ollama.com/capabilities/structured-outputs
- Compare local routing with the existing-account option on setup burden, device resource use, first-run latency, routing quality, and quota cost. A local-only fallback should preserve manual selection if Ollama is unavailable.

## Answer

- Use a small user-configured local Ollama model as the first-release router. It proposes one of the live, ready agent/model/effort tuples for each new user message. Chorus validates its schema and the chosen tuple. The model does not receive attachment bytes, secrets, full history, or tool access; it sees task text, attachment metadata, enabled capability names, and eligible tuples.
- The user sees the proposed agent/model/effort and a short reason. After a five-second countdown, Chorus dispatches automatically. Changing a choice, focusing the review controls, or invoking accessibility interaction pauses the countdown; Send now and Cancel are explicit actions. A manually pinned choice bypasses routing.
- If Ollama is absent, times out, returns malformed/low-confidence output, or names an unavailable tuple, stop automatic dispatch and present the last valid manual target for user confirmation. Do not call Jev or an account-backed router as an implicit fallback. A target that becomes unavailable during the countdown likewise stops dispatch. This local-only fallback is a working assumption inferred from the user's direction; the focused fallback question has not received an explicit reply.
- Route once per new user send; do not recursively route internal handoffs or identical retries. Record proposal, override, final target, router model/version, and fallback reason for explanation and quality review.
- The route selects the first link of Chorus's existing failover chain. Keep the user's account/provider failover preferences, but preflight every fallback link for this turn's attachment and required integration capabilities; skip incompatible links or stop with an explicit error. Queued messages route when they reach dispatch, using then-current candidates.
- Existing-account routing remains a possible later adapter, not first-release default. Validate candidate quality and latency on representative Chorus tasks before enabling local automatic routing by default for all users. See the [implementation spec](../implementation-spec.md#3-local-pre-dispatch-router).
