# Composer v2

Status: ready-for-agent
Blocked by: 01
Parent: ../spec.md
Decided in: [Composer layout](../../chorus-ux-overhaul/issues/08-composer-layout.md) · [Composer paste options](../../chorus-ux-overhaul/issues/04-composer-paste-and-preview.md) · [Rich paste dependency](../../chorus-ux-overhaul/issues/18-rich-paste-dependency.md) · prototype `08-composer-layout.html?variant=A`

## What

- Add `htmd` and `pulldown-cmark` to chorus-core and set workspace `rust-version = "1.85"` (approved). Check the release CI toolchain.
- **Paste:** Swift reads `public.html` (RTF converted to HTML first). A new FFI call converts it to Markdown, with caps on input bytes, output bytes, depth and time; past a cap it falls back to plain text. Hidden text is stripped or made visible. Remote resources are never fetched.
- **Live styling:** Rust returns `pulldown-cmark` spans and Swift applies them to the `NSTextView`. Markers are dimmed.
- **Thumbnails:** `QLThumbnailGenerator` for file URLs, ImageIO `CGImageSourceCreateThumbnailAtIndex` for image bytes. Over-limit files show red with the reason. Show an info line when attachments narrow the router.
- **Layout:** attachments → text → one row (`＋ · mode · preset · permission … 🎙 · model pill · Send`). Send becomes Stop (`⌘.`) while streaming. The box grows to about 220 px, then scrolls.
- Mode and preset controls may be stubs here until issues 09 and 14 land, but must show the decided labels.
- `NSFilePromiseReceiver` drops from Photos, Mail and Safari.

## Done when

- Rust tests: HTML from Slack, Notion and GitHub fixtures converts to the expected Markdown; caps trigger the fallback.
- Check in the app: paste a web page section, a Finder file, a screenshot and a PDF; real thumbnails show; IME Return still doesn't send.
