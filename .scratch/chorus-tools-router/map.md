## Destination

An implementation-ready product spec for macOS Chorus covering session-persistent image and file attachments, a curated user-level integration catalog with per-project enablement, and automatic pre-dispatch selection of an agent/model/reasoning effort using a separately configured router. A local model is a candidate for the router; the first-release backend remains to be decided in issue 06. The router shows its choice and rationale and lets the user override it. Do not call the Jev API directly.

## Notes

- Keep the design local-first and fit it to the current SwiftUI + Rust + ACP architecture.
- Preserve the existing manual agent/model/reasoning controls as overrides.
- Consult current official provider/tool documentation and the repository's codebase-design guidance when resolving tickets.
- Planning ends at a reviewed spec; implementation is a later task.
- Implementation handoff: [Chorus attachments, integration catalog, and local router](./implementation-spec.md).

## Decisions so far

- [Attachment compatibility for current agent bridges](./issues/01-attachment-compatibility.md): Current turn transport is text-only; provider-specific ACP capabilities and file limits need runtime handling.
- [Integration catalog options](./issues/02-integration-catalog-research.md): MCP fits the current session path; CLI hooks and proxy modes need distinct support and controlled install effects.
- [Router feasibility with existing agent accounts](./issues/03-router-existing-agent-research.md): A separate router inference is feasible with existing accounts, but uses quota and latency; effort choices need capability discovery.
- [Attachment lifecycle and provider fallback](./issues/04-attachment-policy.md): Keep originals per session; send images, text/code, and text PDFs within 5 files / 20 MB each / 50 MB total; later agents fetch needed files, and deletion revokes access.
- [Integration catalog trust and permissions](./issues/05-integration-policy.md): Curated MCP, skill, and callable CLI entries install once, enable by project, and update automatically only to verified approved versions with rollback.
- [Pre-dispatch router policy and UX](./issues/06-router-policy.md): A small local Ollama selector proposes a ready agent/model/effort tuple, shows a countdown with override, and falls back to confirmed manual selection on failure.

## Not yet specified

- None for the first-release product spec. Bridge-version compatibility, routing quality, and installer health are implementation verification gates in the [spec](./implementation-spec.md).

## Out of scope

- Calling the Jev decision API directly.
- Windows and non-macOS clients for this effort.
