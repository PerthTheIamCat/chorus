# Attachment lifecycle and provider fallback

Type: grilling
Status: resolved
Blocked by: 01

## Question

Given the attachment compatibility research, settle the first-release attachment contract: supported file and image types, per-file and per-turn limits, whether unsupported types are rejected or converted, what content the receiving agent sees, and how the user inspects/removes stored attachments. The agreed direction is to persist attachment bytes in Chorus session storage and make them available across handoffs; decide the remaining lifecycle and fallback details without changing that direction silently.

## Answer

- Chorus keeps original attachment bytes in session storage and links them to the user message. The first release sends common images, text/code files, and PDFs with extractable text. Unsupported files may remain in the session but must be marked unreadable by the selected agent. Images may be normalized for a bridge; PDFs are extracted locally to text, with the conversion disclosed. Office files, archives, audio/video, and OCR for scanned PDFs are not first-release agent inputs.
- Limit each message to five files, 20 MB per file, and 50 MB total. Validate before import/dispatch and apply any stricter live bridge/model limit. An unsupported or over-limit file blocks dispatch with a specific explanation; never silently omit it.
- On handoff, show a session attachment manifest and let the new agent request only the older files it needs through a session-scoped Chorus retrieval tool. Do not resend every historic file automatically. The current message's files use capability-gated ACP content blocks or bounded text.
- The composer and history show attachment name, type, size, conversion, and availability. Deleting a sent attachment immediately revokes further use; history retains a deleted marker, and bytes are collected when no live relation refers to them. A `@file` workspace reference remains distinct from a stored attachment.
- Implementation detail and acceptance cases live in the [implementation spec](../implementation-spec.md#1-attachments). The [compatibility report](../research/01-attachment-compatibility.md) distinguishes bridge capability from provider API support.
