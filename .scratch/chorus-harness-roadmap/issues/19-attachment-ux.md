# Attachment UX polish

Type: grilling
Status: resolved
Blocked by: 01, 13

## Question

The attachment policy is decided (see `chorus-tools-router` ticket 04). What is the UX gap compared with the best harnesses? Decide on paste and drag targets (composer, thread, file tree), previews and thumbnails, pasting screenshots straight from the clipboard, attaching selected code or diff hunks as context, a per-agent indicator of what each agent can actually read, and the limit and error states.

## Answer

- **Clipboard Image & File Paste (`⌘V`)**:
  - Pressing `⌘V` with an image/file in the macOS pasteboard immediately stages an attachment chip in a bar floating above the composer.
  - Rendered with premium native SF Symbols (`photo`, `doc.text`) rather than emojis, displaying filename and size badges.
  - Pressing `Spacebar` or clicking triggers native macOS Quick Look for instant full-resolution preview.
- **Full Drag-and-Drop Dropzones**: Both the Composer and Center Stage conversation stream act as active drop targets with smooth accent-tinted borders and a clear *"Drop files to attach to turn"* overlay.
- **Context Staging via Shortcut (`⌘L`)**: In the Code Viewer or Diff Inspector, selecting lines and pressing `⌘L` (or right-click) stages an interactive reference chip in the composer (e.g. `[@router.rs:310-325]`) without polluting the prompt text with raw duplicate code.
- **Proactive Vision & Compatibility Indicators**:
  - Displays native SF Symbol status badges next to the model selector: `eye` (Vision ready) for multimodal models (Claude 3.7 Sonnet, Codex), and `exclamationmark.triangle` (Text-only) for small text-only local models.
  - When an image is attached to a text-only target, provides a 1-click prompt to switch to a vision-capable model or run via router.
- **Strict Limit Feedback**: Enforces the 5 files / 20MB per file / 50MB total storage quota with clean, non-intrusive inline toasts rather than modal error alerts.
