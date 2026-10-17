# Research 04: composer paste and preview

Ticket: [../issues/04-composer-paste-and-preview.md](../issues/04-composer-paste-and-preview.md) · Researched 2026-10-17 · Host: macOS 27.0.1, rustc 1.92, app target `.macOS(.v14)`

## Question

1. How can rich text pasted from a browser, Slack or Notion (HTML/RTF) become Markdown?
2. How can the composer show that Markdown nicely and still be editable?
3. How do we read images and files from `NSPasteboard` and show real thumbnails (QuickLookThumbnailing, `NSImage`), including PDFs and large files?
4. Which approaches and libraries fit (licence, size, upkeep)?
5. How do ChatGPT, Claude desktop, Slack and Linear lay out their composers?

## TL;DR / recommendation

- **HTML to Markdown belongs in Rust, using `htmd`** (Apache-2.0, html5ever only, passes turndown.js's test suite). Swift reads the `public.html` flavour, or the RTF flavour turned into HTML by AppKit, and passes the string over FFI. This follows CLAUDE.md 2.3 (no logic in Swift) and means no WebKit and no JS engine. Catch: `htmd` 0.5 uses Rust **edition 2024**, which needs Rust 1.85 or later. The workspace says `rust-version = "1.82"`. Adding the dependency and raising the MSRV both need Perth's go-ahead (CLAUDE.md 11).
- **Composer stays a plain-text `NSTextView` holding the Markdown source, with live styling.** Rust parses the draft with `pulldown-cmark`'s `into_offset_iter` and returns `(range, kind)` spans. Swift applies attributes (bold, mono, dimmed markers) to the `NSTextStorage` and never changes the characters. The Markdown text stays the truth. This works on macOS 14. It avoids the GPL `STTextView` and the macOS 26-only rich `TextEditor`, and it needs no round-trip from AttributedString back to Markdown, which Apple does not provide.
- **Thumbnails: `QLThumbnailGenerator.generateRepresentations` for anything that has a file URL** (images, PDFs, text, video). It shows an icon first and swaps in the real thumbnail later. **Pasted image bytes with no URL go through `CGImageSourceCreateThumbnailAtIndex` with `kCGImageSourceThumbnailMaxPixelSize`**, so the full image is never decoded just to draw a 48 px chip.
- **Paste order must change: file URLs before image data, and HTML before plain text.** Read only inside the user's paste or drop, so macOS 15.4+ pasteboard-privacy alerts never fire. Add `NSFilePromiseReceiver` for drags from Photos, Mail and Safari.
- **Layout pattern shared by Claude and Slack (confirmed in their own docs):** the attach button sits at the **left** of the composer, and the model/effort control sits **next to Send on the right**. Linear puts attachments behind `/` commands and Cmd+Shift+U. Ticket 08 should take this as its starting point.

---

## 1. Rich paste to Markdown

### What the pasteboard gives us

- `NSPasteboard.PasteboardType.html` is HTML content, and Apple notes that "An `NSTextView` object can read HTML data, but not write it" ([Apple: PasteboardType.html](https://developer.apple.com/documentation/appkit/nspasteboard/pasteboardtype/html)). Browsers, Slack and Notion normally write `public.html` next to `public.utf8-plain-text`. *Not checked per app in this pass: inspect `NSPasteboard.general.types` after copying from each one before building.*
- Today `ChatNSTextView` sets `isRichText = false` (`apps/macos/Sources/Chorus/Composer.swift`, `makeNSView`), so a rich paste falls through to `super.paste` and keeps only the plain-text flavour. Headings, links and lists are lost.
- The place to hook in: `NSTextView.readSelection(from:type:)` is the documented override "to read pasteboard types other than the default types". Use `rangeForUserTextChange` for the range it replaces ([Apple: readSelection(from:type:)](https://developer.apple.com/documentation/appkit/nstextview/readselection(from:type:))). Overriding there, rather than in `performKeyEquivalent`, also covers Edit > Paste, Services and undo grouping.

### Options

| Option | How | Licence / size | Upkeep (GitHub, 2026-10-17) | Verdict |
|---|---|---|---|---|
| **`htmd` (Rust)** | `htmd::convert(html)`; options and skip-tags match turndown | Apache-2.0; deps `html5ever`, `markup5ever_rcdom`, `phf` ([Cargo.toml](https://github.com/letmutex/htmd/blob/main/Cargo.toml)) | 0.5.6, pushed 2026-10-15, about 5.3 M recent downloads ([crates.io](https://crates.io/crates/htmd)) | **Pick.** "passes all test cases of turndown.js", converts tables, about 16 ms for a 1.37 MB page on M4 ([README](https://github.com/letmutex/htmd)). Edition 2024 means a MSRV bump. |
| `html2md` (Rust) | | **GPL-3.0+** ([crates.io](https://crates.io/crates/html2md)) | 0.2.17 | Reject: licence. |
| Demark (Swift) | Turndown.js inside a `WKWebView`, or `html-to-md` on JavaScriptCore | MIT | pushed 2026-10-12 ([repo](https://github.com/steipete/Demark)) | Works, but it is logic in Swift, adds a WebView or JS engine, and its API is `@MainActor` async. |
| `NSAttributedString(html:)` then walk the attributes to Markdown | AppKit importer | built in | — | Apple **discourages** the synchronous HTML initialisers and points to the out-of-process WebKit `loadFromHTML` family "for improved security, stability, and code isolation" ([Apple: DocumentType.html](https://developer.apple.com/documentation/foundation/nsattributedstring/documenttype/html), [init(html:…)](https://developer.apple.com/documentation/foundation/nsattributedstring/init(html:options:documentattributes:))). We would still have to write our own attributes-to-Markdown exporter. Reject. |
| SwiftSoup + our own converter | DOM in Swift | MIT, active ([repo](https://github.com/scinfu/SwiftSoup)) | | Rewrites what `htmd` already does, in the wrong layer. |

**RTF-only sources** (TextEdit, Pages, some Office apps): read `.rtf` into an `NSAttributedString`, export it as HTML with `data(from:documentAttributes: [.documentType: .html])`, and send that through the same `htmd` path. One converter covers both. RTF import is not the HTML importer Apple warns about. *Not tested in this pass.*

**Guards for the Rust side:** cap the input size (the MCP body is already capped at 64 KB). Skip `script`, `style`, `meta` and the Slack/Notion wrapper `span`s. If the plain-text flavour is shorter than about 1.2× the converted Markdown and the HTML had no block tags, use the plain text, so pasting one word does not come out as `**word**`. Holding Option or Shift (Cmd+Option+Shift+V, "Paste and Match Style") should always paste plain text. That is the standard macOS paste-plain shortcut; *confirm it against the HIG in ticket 08.*

## 2. Showing Markdown nicely while it stays editable

| Approach | macOS 14? | Notes |
|---|---|---|
| **Raw Markdown in `NSTextView` with live attribute styling** | yes | Text stays plain, so the source of truth is never in doubt. Parse in Rust with `pulldown-cmark` (MIT, 0.13.4), whose `into_offset_iter` yields `(Event, Range)` pairs that map back to source ranges ([docs.rs](https://docs.rs/pulldown-cmark/latest/pulldown_cmark/struct.Parser.html)). `htmd` already uses pulldown-cmark as a dev-dep. Swift applies fonts and colours to ranges only, which is drawing and so allowed under 2.3. Re-run on `textDidChange`, debounced. A 2–4 KB draft parses in well under a frame. |
| SwiftUI `TextEditor(text: Binding<AttributedString>, selection:)` | **no, macOS 26.0+** | ([Apple](https://developer.apple.com/documentation/swiftui/texteditor/init(text:selection:)-11r0a), WWDC25 sample [Building rich SwiftUI text experiences](https://developer.apple.com/documentation/swiftui/building-rich-swiftui-text-experiences)). WYSIWYG only. Apple's `AttributedString(markdown:)` parses Markdown in (`.full` "interprets the full Markdown syntax", macOS 12+, [Apple](https://developer.apple.com/documentation/foundation/attributedstring/markdownparsingoptions/interpretedsyntax-swift.enum/full)), but there is no exporter back out, so sending Markdown means writing a lossy serialiser. Reject for now. |
| `STTextView` (TextKit 2) | yes | **GPL-3.0 or paid commercial licence** ([LICENSE](https://github.com/krzyzanowskim/STTextView/blob/main/LICENSE.md)). Reject unless Perth buys a licence. |
| `CodeEditTextView` | yes | MIT, active ([repo](https://github.com/CodeEditApp/CodeEditTextView)), but built for code (gutter, line numbers). Too heavy for a chat box. |
| Separate "Preview" toggle rendered with Textual / MarkdownUI | yes | MarkdownUI is "Maintenance mode — new development in Textual" ([repo](https://github.com/gonzalezreal/swift-markdown-ui)). Textual is MIT, pushed 2026-06 ([repo](https://github.com/gonzalezreal/textual)). Read-only and an extra dependency, so it fits the **chat thread** better than the composer. |
| Neon (styling engine) | yes | BSD-3, tree-sitter based ([repo](https://github.com/slsrepo/Neon)). More machinery than a Markdown draft needs. |

Pick the first row: show Markdown markers dimmed (secondary label colour) rather than hidden, so cursor movement stays predictable. Bold and italic as real font traits. Inline code and fences in mono with a tinted background. Links underlined. Nothing is hidden, so there is no mismatch between what the caret sees and what the source holds.

## 3. Images and files from the pasteboard, and thumbnails

### Reading

- **Privacy (macOS 15.4+).** `NSPasteboard.accessBehavior` defaults to *ask on programmatic access* for the general pasteboard. But "access that is both user originated and paste related will always be allowed, and will not result in a notification" ([Apple: AccessBehavior.ask](https://developer.apple.com/documentation/appkit/nspasteboard/accessbehavior-swift.enum/ask), [.default](https://developer.apple.com/documentation/appkit/nspasteboard/accessbehavior-swift.enum/default)). So: read only inside `paste`/`readSelection`/drop. Never poll the clipboard, for example to show a "paste your screenshot" hint. If a hint is wanted, `detectedPatterns(for:)` checks the first item "without notifying the person" ([Apple](https://developer.apple.com/documentation/appkit/nspasteboard/detectedpatterns(for:))). Gate it with `#available(macOS 15.4, *)`.
- **File URLs:** `readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])` limits the result to file URLs ([Apple](https://developer.apple.com/documentation/appkit/nspasteboard/readingoptionkey/urlreadingfileurlsonly)). Today's code filters `isFileURL` by hand, which works the same.
- **File promises:** drags from apps that don't have a file on disk yet (Photos, Mail, Safari images) arrive as `NSFilePromiseReceiver` on the drag pasteboard ([Apple](https://developer.apple.com/documentation/appkit/nsfilepromisereceiver)). Neither `performDragOperation` nor the SwiftUI `onDrop` handles them today, so those drops fail silently.
- **Order bug to fix (code reading, not yet reproduced):** `checkPasteboardForMedia()` tries `.png`, then `.tiff`, then `NSImage(pasteboard:)`, and only **after** those the file URLs. When a copy carries both a file URL and an image representation (Finder is the likely case: it puts the file's icon on the pasteboard), we would attach a picture of the icon instead of the file. *Verify by checking `NSPasteboard.general.types` after Cmd+C on a file in Finder.* Either way, the order should be **file URL → file promise → image data → HTML → RTF → plain text**.
- There is also **dead code**: `ComposerField.pasteFromClipboard()` and `handle(_:_:)` duplicate the `ChatNSTextView` logic and nothing calls them.

### Thumbnails

- **`QLThumbnailGenerator`** (macOS 10.15+) handles images, Live Photos, text, **PDF**, audio and video ([Apple: QuickLookThumbnailing](https://developer.apple.com/documentation/quicklookthumbnailing)). `generateRepresentations(for:update:)` "create[s] a file icon or low-quality thumbnail quickly, and replace[s] it with a higher quality thumbnail once it's available" ([Apple article](https://developer.apple.com/documentation/quicklookthumbnailing/creating-quick-look-thumbnails-to-preview-files-in-your-app)). That fits a chip that should appear instantly. Request it with `Request(fileAt:size:scale:representationTypes: .all)`, `iconMode = false` for a raw undecorated thumbnail ([Apple](https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailgenerator/request/iconmode)), and the window's `backingScaleFactor`. Do not set `minimumDimension`: if the system can't reach it, "QLThumbnailGenerator doesn't generate a thumbnail" ([Apple](https://developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailgenerator/request/minimumdimension)).
- **It needs a file URL.** File imports have one, so thumbnail from the user's original URL before or while importing. For pasted bytes, use **ImageIO**: `CGImageSourceCreateWithData` + `CGImageSourceCreateThumbnailAtIndex` with `kCGImageSourceThumbnailMaxPixelSize`. Without that key the thumbnail "may be as big as the image itself" ([Apple](https://developer.apple.com/documentation/imageio/kcgimagesourcethumbnailmaxpixelsize)). ImageIO can also thumbnail a PDF page at 72 dpi when you pass `kCGImageSourceCreateThumbnailFromImageIfAbsent`/`Always` ([Apple](https://developer.apple.com/documentation/imageio/cgimagesourcecreatethumbnailatindex(_:_:_:))).
- **Avoid `NSImage(data:)` / `NSBitmapImageRep` just to draw a preview.** They decode the whole image. Today `prepareAttachment` does this for non-PNG/JPEG images, which is fine for normalising but should not be reused for chips.
- **Large files.** The core rejects attachments over `MAX_FILE_BYTES` (20 MiB, `crates/chorus-ffi/src/lib.rs` `chorus_import_attachment`). But `Engine.importFile` maps the whole file and runs `PDFDocument(data:).string` before Rust says no. Check the size first with `URLResourceValues.fileSize` and show the "too big" chip before reading any bytes. QL thumbnailing reads only what it needs, so a 500 MB video still gets a preview chip.
- **Full preview on click:** `QLPreviewView` (Quartz, macOS 10.6+) can be embedded, or use the system Quick Look panel ([Apple](https://developer.apple.com/documentation/quicklookui/qlpreviewview)). Both are drawing only, so 2.3 still holds.
- Thumbnails are a Hot-only UI artefact. Keep them in an in-memory `NSCache` keyed by attachment id. They never go into the ledger or blob store (CLAUDE.md 2.1).

## 4. How leading apps lay out the composer

Only claims found in each vendor's **own** help pages are listed. Third-party teardowns were left out.

| App | Attach | Model / mode | Send | Paste / format | Source |
|---|---|---|---|---|---|
| **Claude** | "+" button "in the lower left corner of the chat box" → "Add files or photos"; drag into the window; paste images | "The model menu next to the send button" controls model, effort and the Thinking/Extended toggle; "The selected model and effort level appear next to the send button" | right | images by paste; limits 500 MB/file, 20 files/chat | [Upload files](https://support.claude.com/en/articles/8241126-upload-files-to-claude), [Model, effort and thinking](https://support.claude.com/en/articles/8664678-change-the-model-effort-and-extended-thinking-settings) |
| **Slack** | "plus icon next to the message field"; drag in up to 10 files; previews can be reordered before sending | — | — | formatting toolbar "visible by default" in the message field, toggled by a formatting icon; markup mode hides it | [Add files](https://slack.com/help/articles/201330736-Add-files-to-Slack), [Format messages](https://slack.com/help/articles/202288908-Format-your-messages-in-Slack) |
| **Linear** | `/file` or `/insert`, Cmd+Shift+U | — | — | pasted Markdown "will be converted into rich text automatically"; pasted links become embeds, Esc / "Keep as link" undoes | [Editor docs](https://linear.app/docs/editor) |
| **ChatGPT** | *not verified*: help.openai.com returned 403 to fetches | the official macOS release notes, seen only as a search snippet, say models can be switched "directly from the chat bar" (2024) | — | — | [macOS release notes](https://help.openai.com/en/articles/9703738-macos-app-release-notes) (snippet only) |

Read-across for ticket 08: **left = add (attach, `/`, `@`), right = model chip + Send**. Mode toggles sit inside the model menu (Claude) rather than in a separate row. Attachment previews sit above the text and can be reordered (Slack). Linear shows the other way of doing it (WYSIWYG, attach via command). It does not fit a Markdown-native agent prompt, but its "Esc undoes the paste conversion" is worth copying: after a rich paste, Esc or Cmd+Z should restore the plain text.

## 5. Proposed pipeline (for ticket 08 / implementation)

```
paste/drop ─▶ ChatNSTextView.readSelection(from:type:)          (Swift: read only)
   fileURL / filePromise ─▶ size check ─▶ Engine.importFile      ─▶ QL thumbnail(url)
   image data            ─▶ Engine.importImage                   ─▶ ImageIO thumbnail(data)
   html | rtf→html       ─▶ chorus_html_to_markdown(html, plain) ─▶ insert at rangeForUserTextChange
   plain                 ─▶ super
textDidChange ─▶ chorus_markdown_spans(text) ─▶ apply attributes  (Rust parses, Swift paints)
```

New FFI (pure, no engine handle): `chorus_html_to_markdown` and `chorus_markdown_spans`. Both are JSON in and out, and both are panic-guarded like the existing FFI.

## Open questions / not verified

- Which flavours Slack, Notion, Chrome, Safari and Finder actually write. Run a one-off `NSPasteboard.general.types` dump per app.
- Whether to raise the workspace MSRV 1.82 → 1.85 for `htmd`. The other route is pinning an older `htmd` built on edition 2021: *not checked whether one exists.*
- ChatGPT composer layout from a primary source (the help centre blocks automated fetches). Check by hand in `/Applications/ChatGPT.app` if ticket 08 needs it.
- Performance of re-styling on every keystroke for very long drafts (over 20 KB). Measure before adding incremental parsing.
