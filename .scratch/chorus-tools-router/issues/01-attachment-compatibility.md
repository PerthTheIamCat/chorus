# Attachment compatibility for current agent bridges

Type: research
Status: resolved

## Question

Which currently supported Chorus agent/ACP bridge combinations can receive images and common files, and what does each accept at the transport boundary? Trace the current Swift composer, C FFI, session ledger/blob storage, and ACP bridge. Use current primary documentation to distinguish native binary/media attachment support from text extraction or workspace-path workarounds. Report unsupported formats, size limits if documented, and a source-backed compatibility matrix. Do not implement changes or use credentials/live accounts.

## Answer

The current composer → FFI → Rust session → ACP path sends text only. ACP has image and embedded-resource content blocks, but Chorus does not yet parse bridge capabilities or persist attachment records; the generic blob store is not wired for attachments. Codex and Claude bridges advertise conditional image/resource support, while Gemini's ACP invocation and capabilities need runtime/version checks. Provider API size/type limits are not interchangeable with ACP CLI limits, so implementation should capability-gate and report unsupported cases rather than assume a universal file contract. See the [compatibility report](../research/01-attachment-compatibility.md).
