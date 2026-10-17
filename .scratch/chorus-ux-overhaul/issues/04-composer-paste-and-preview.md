# Composer paste and preview options

Type: research
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:research

## Question

On macOS (SwiftUI/AppKit), how can pasted rich text (HTML/RTF from a browser, Slack, Notion) become Markdown, and how can the composer show that Markdown nicely while staying editable? How do you read images and files from `NSPasteboard` and show real thumbnails (QuickLookThumbnailing, `NSImage`), including PDFs and large files? Compare approaches and libraries (licence, size, upkeep), and note how leading chat apps (ChatGPT, Claude desktop, Slack, Linear) lay out their composer: where attachments, model picker, mode toggles, and send sit.

Research: ./research/04-composer-paste-and-preview.md

## Answer

- **Paste:** Swift reads `public.html` (RTF is turned into HTML first). Rust converts the HTML to Markdown with `htmd` (Apache-2.0).
- **Editor:** a plain `NSTextView` holds Markdown as the source of truth. Rust parses it with `pulldown-cmark` and returns styling spans; Swift only applies them.
- **Thumbnails:** `QLThumbnailGenerator` for a file URL, ImageIO `CGImageSourceCreateThumbnailAtIndex` for image bytes.
- **Rejected:** `html2md` (GPL), `NSAttributedString(html:)`, `STTextView` (GPL or paid), rich `TextEditor` (macOS 26 only).
- **Bugs found:** the image check runs before the file-URL check, so a Finder copy may attach its icon instead of the file (from reading the code, not reproduced). Big files are mapped into memory before the 20 MiB limit is checked. `NSFilePromiseReceiver` drops are not handled. `pasteFromClipboard()` and `handle(_:_:)` are dead code.
- **Layout norm:** attach on the left, model/effort next to Send on the right.
- **Needs Perth:** the `htmd` dependency plus raising `rust-version` 1.82 to 1.85 (see Rich paste converter dependency).

Detail: [research](../research/04-composer-paste-and-preview.md).
