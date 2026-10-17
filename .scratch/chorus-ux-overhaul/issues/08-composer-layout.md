# Composer layout

Type: prototype
Status: resolved
Blocked by: 01, 18
Parent: ../map.md
Labels: wayfinder:prototype

## Question

What should the composer look like: where the text, attachment thumbnails, model/route preset/orchestra controls, and send button go, how pasted Markdown is shown, and how it grows and shrinks? Follow the conventions of leading chat apps.

Constraints from Security findings triage: follow research 03 §4 "Pasting HTML" and "Previewing pasted files".

Prototype: ./prototypes/08-composer-layout.html (variants A/B/C, states empty/paste/stream)

## Answer

Perth picked **variant A, one control row** (2026-10-17). See [prototype](../prototypes/08-composer-layout.html) `?variant=A`.

- **Order, top to bottom:** attachment thumbnails, then the text, then one control row.
  - **Thumbnails:** a real image thumbnail, a PDF first page, file chips with a type icon and size. Over-limit files show red with the reason. An info line appears when an attachment narrows the router's choice, for example skipping models that can't read images.
  - **Text:** Markdown is the source, styled live by Rust spans: headings, lists, inline code, highlighted code blocks. Markers are dimmed, not hidden.
  - **Controls, left:** `＋` attach · mode (`Orchestra` / `Auto route`) · route preset (`ประหยัด/สมดุล/เต็มกำลัง`) · permission (`ถามก่อน/ให้ทำเลย`).
  - **Controls, right:** dictation · model pill (in Orchestra it shows "นำโดย <orchestrator>"; in Auto route it shows the route proposal) · Send.
- **States:**
  - Empty: Send is disabled.
  - Streaming: Send becomes Stop (`⌘.`), and the box stays usable for steering.
  - Focus ring on the box.
- **Hint line under the box:** `↩` send · `⇧↩` newline · `⌘V` paste image or file. Return does nothing during IME composition.
- **Growth:** the box grows with the content up to about 220 px, then scrolls inside.
- **Security:** the triage rules for HTML paste and file previews apply.
