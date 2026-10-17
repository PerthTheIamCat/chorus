# Research 06: File-type icon sources

Ticket: [../issues/06-file-type-icons.md](../issues/06-file-type-icons.md) · Researched 2026-10-17

## Question

Where can the macOS shell get accurate per-file-type icons for the workspace tree (`.rs`, `.md`, `.swift`, `.ts`, `.json`, `.toml`, lockfiles, `Dockerfile` and so on)? Compare licence, coverage, size, light/dark support and how extensions and special file names map to icons, then pick one.

## Where we are today

`WorkspaceExplorerView.fileIcon(_:)` (`apps/macos/Sources/Chorus/WorkspaceExplorerView.swift:333`) maps five suffixes to SF Symbols and sends everything else to `doc`. `.json` and `.toml` share `gearshape`. `.rs` gets the generic `chevron.left.forwardslash.chevron.right`. Lockfiles, `Dockerfile`, `README` and `LICENSE` have no icons of their own.

`ProviderIcon.swift` already shows a pattern that works: SVGs bundled in `Sources/Chorus/Resources` (SwiftPM `.process("Resources")`), loaded with `Bundle.module.path(forResource:ofType:"svg")` + `NSImage(contentsOfFile:)`, and an SF Symbol fallback.

## Options

### 1. System icons: `NSWorkspace.icon(for: UTType)`

- API: `func icon(for contentType: UTType) -> NSImage`, macOS 11.0+, and it "returns a default icon if the operation fails" [A1]. `icon(forFile:)` returns a 32×32 image and is safe to call from any thread [A2].
- **The type lookup is wrong for developer files.** I measured this on this machine (macOS 27.0.1) with `UTType(filenameExtension:)`, then rendered and hashed each icon at 32 px:

  | ext | UTType resolved | icon |
  |---|---|---|
  | `rs` | `dyn.ah62d4rv4ge81e62` (dynamic, undeclared) | blank document |
  | `ts` | `public.mpeg-2-transport-stream` (a **video** type) | the same blank icon as `rs` |
  | `toml` | `public.toml` | the same blank icon as `rs` |
  | `lock` | dynamic | generic |
  | `yaml`, `py`, `txt` | declared | all identical to the `json` icon |

  Across 14 extensions there were only 9 distinct icons. TypeScript is the worst case: macOS treats `.ts` as video. Results also depend on which apps are installed, because apps declare UTIs (Xcode adds some, for example). Two machines can therefore show different trees.
- Special file names (`Dockerfile`, `Cargo.lock`, `LICENSE`) have no extension, so a UTType lookup can't tell them apart.
- Style: full-colour page-shaped document icons. These look out of place at 13–16 pt beside SF Symbol folders.
- Licence: system-provided, so there are no redistribution concerns.

**Verdict:** wrong (`.ts`), inconsistent between machines, and blind to file names. Not usable as the main source.

### 2. SF Symbols

- More than 7,000 symbols [A3]. Usage terms prohibit "using symbols — or images that are confusingly similar — in app icons, logos, or any other trademarked use". Some symbols can't be customised, and the SF Symbols app shows their usage restrictions [A4]. Using them as in-app UI glyphs is the intended use.
- Coverage: generic shapes only (`doc.text`, `curlybraces`, `gearshape`, `lock`, `swift`). There are no Rust, TypeScript, Docker, TOML or Markdown marks. That is the reason the current tree can't tell `.json` from `.toml`.
- Light/dark: automatic (template rendering).

**Verdict:** the right fallback and the right style for folders and chrome, but it can't tell file types apart.

### 3. Bundled icon sets (VS Code icon themes and Devicon)

Licence text was read from each repo's LICENSE file. Counts and sizes come from the GitHub git-trees API for the default branch on 2026-10-17 (all SVG files under `icons/`).

| Set | Licence | Icons (SVG) | Total bytes | ViewBox / style | Light/dark | Mapping source | Last push |
|---|---|---|---|---|---|---|---|
| **Catppuccin Icons** | MIT, © 2023 Catppuccin, thang-nm [L1] | 656 per flavour (428 file + 228 folder) × 4 flavours + `css-variables` | 2,138,240 total, about 428 KB per flavour | 16×16, **stroke-only, one colour per icon** | **Latte (light) + Frappé/Macchiato/Mocha (dark)**, the same glyph per flavour [R1] | `src/defaults/fileIcons.ts`: `{ '<icon>': { fileExtensions, fileNames, languageIds } }` | 2026-10-14 |
| Material Icon Theme | MIT, © 2025 Material Extensions [L2] | 906 (636 file + 270 folder) | 711,901 | 32×32 (some 16×16), filled, multicolour | 50 `_light` variants, chosen by a `light: true` flag | `src/core/icons/fileIcons.ts`: `{ name, fileExtensions, fileNames }` | 2026-10-16 |
| vscode-icons | Code MIT [L3]; **icons CC BY-SA 4.0; "Branded icons are licensed under their copyright license"** [R2] | 1,706 | 3,812,010 | 32×32, gradients, multicolour | 191 `file_type_light_*` | `src/models/contributions/associations.ts` | 2026-10-16 |
| Seti UI | MIT, © 2014 Jesse Weed [L4] | 169 | 175,778 | single colour, 32×32 frame [R3] | none (colour comes from LESS) | `styles/components/icons/mapping.less` (LESS mixins) | 2025-10-28 |
| Devicon | MIT, © 2015 konpa [L5]; README: "All product names, logos, and brands are property of their respective owners" [R4] | 1,877 (about 6 variants per technology) | 9,467,495 | technology **logos**, not file types | plain/original/line variants | no extension mapping | 2026-10-12 |

Coverage of the ticket's list (searched by icon name in each tree):

| | `.rs` | `.md` | `.swift` | `.ts` | `.json` | `.toml` | `Cargo.toml` | `Cargo.lock` | `*.lock` | `Dockerfile` | README / LICENSE |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Catppuccin | ✓ | ✓ | ✓ | ✓ (+`d.ts`, test, config) | ✓ | ✓ | ✓ `cargo` | ✓ `cargo-lock` | icon exists, no extension mapping | ✓ | ✓ / ✓ |
| Material | ✓ | ✓ | ✓ | ✓ (+`d.ts`) | ✓ | ✓ | → toml | → `lock` | ✓ | ✓ | ✓ / ✓ |
| vscode-icons | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ `cargo` | no lock icon | — | ✓ | ✓ / ✓ |
| Seti | ✓ | ✓ | ✓ | ✓ | ✓ | — | — | — | `lock` | ✓ | — / ✓ |
| Devicon | logos for Rust, Swift, TS, Docker; no file-type semantics | | | | | | | | | | |

Notes on each set:

- **vscode-icons**: CC BY-SA 4.0 is copyleft for the artwork. Any icon we tint or edit has to be shared under the same terms, and attribution is required. Branded icons add a third licence on top. That is too much licensing to keep track of for a bundled app.
- **Devicon**: answers "which technology is this", not "which file is this". It has no mapping, nothing for lockfiles, README or LICENSE, and it is the heaviest set.
- **Seti**: small but sparse (no TOML, no YAML, no light variants), and its mapping lives in LESS, which is awkward to extract.
- **Material**: the strongest coverage per byte, with a clean TS mapping. But its filled multicolour 32-grid glyphs look like VS Code next to SF Symbols, and it ships only 50 light variants, so most icons have no light-mode tuning.
- **Catppuccin**: 16×16 stroke glyphs with one colour each are the closest match to SF Symbols' monochrome line style. Every icon exists in a light flavour (Latte) and dark flavours (Mocha and others) with identical shapes and different palettes, for example `rust.svg` uses stroke `#fe640b` in Latte. It maps special file names explicitly (`cargo.toml`, `cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`, `dockerfile`, `readme.md`, `copying`…).

### Mapping rules worth copying

VS Code's icon-theme contract defines the precedence that every one of these sets is authored against [V1]:

- "File name matching is case insensitive." No wildcards.
- Extensions are "a file name segment after a dot", compared case-insensitively. `lib.d.ts` matches both `d.ts` and `ts`, so try the longest one first.
- Order: "file name match with parent > file name match > file extension match with parent > file extension match > language match".

**Trap:** sets also map through `languageIds`, which VS Code resolves through its own language contributions. A consumer outside VS Code gets no matches from those keys. I checked: in Catppuccin, 1 of 392 file entries (`diff`) is `languageIds`-only. Material keeps them in a separate `languageIcons.ts`, but its main entries (`typescript`, `json`, `markdown`) also list `fileExtensions`. To be safe, the generated table should include explicit extensions for anything left only under `languageIds`.

### Rendering in this app

`NSImage(data:)` decodes SVG natively. On this machine a 16×16 test SVG decoded as `_NSSVGImageRep` at size 16×16. `ProviderIcon` already loads bundled SVGs this way, so no new dependency is needed. Colour icons must **not** set `isTemplate = true` (that would flatten them to monochrome), unlike the provider marks.

## Trademarks (applies to every bundled set)

MIT covers the copyright in the drawings, not the marks. Rust, Docker, TypeScript and similar icons depict third-party trademarks. Devicon says so outright [R4], and vscode-icons gives branded icons their own licence [R2]. Showing a mark next to a file of that type is nominative use, but the icons must stay inside the file tree, never in Chorus's app icon or branding. That is the same line Apple draws for SF Symbols [A4]. MIT also requires keeping the copyright notice in copies: ship the LICENSE text in the bundle and credit it in About.

## Recommendation

**Use Catppuccin Icons, with Latte for light appearance and Mocha for dark, as a curated subset. Keep SF Symbols for folders and as the fallback for unmapped files. Do not use `NSWorkspace` type icons.**

Why: it is plain MIT with no CC/copyleft layer, it has the closest visual match to SF Symbols, it is the only set with a full light/dark pair for every icon, and it has the best special-file-name coverage of the five (`Cargo.toml`, `Cargo.lock`, many lockfiles, `Dockerfile`, README, LICENSE). Material Icon Theme is the runner-up if the shell later moves to filled colour icons.

How to apply it (for the implementation ticket, not done here):

1. Curate around 60–100 icons for the languages Chorus users touch. That costs about 40–65 KB per flavour if the 428 KB / 656-icon flavour average holds. Pin to a tagged release (package `catppuccin-vsc-icons` 1.26.0 at time of writing) and ship its LICENSE.
2. Generate one table from `src/defaults/fileIcons.ts` (`fileNames` → icon, `fileExtensions` → icon), add explicit extensions for `languageIds`-only entries, and add `lock` → `lock` (the icon exists but has no extension mapping).
3. **Resolve the icon in Rust**, using the VS Code precedence above: file name, then longest extension, then fallback. Return an `icon` key on each tree node from the FFI. Swift only maps key → `<key>.svg` in the current appearance's flavour. Per CLAUDE.md 2.3, this keeps the lookup rules out of Swift, so a WinUI shell reuses them unchanged.
4. Unknown key or missing asset → today's SF Symbol `doc`. Folders stay on SF Symbols `folder` / `folder.fill`.

## Not verified

- I did not render Catppuccin glyphs at 13 pt in the actual tree or check contrast against the sidebar material. A prototype screenshot should confirm that Latte and Mocha both read well.
- I did not test SVG decoding through an asset catalog in a SwiftPM `.process` build; only raw `NSImage(data:)` was tested.
- The exact SF Symbols licence text (shipped inside the SF Symbols app) wasn't read; the HIG summary [A4] is the source cited.
- Coverage was checked by icon names in the trees and by spot-reading the mapping files, not by running every extension through a resolver.

## Sources

- [A1] Apple, `NSWorkspace.icon(for:)`: https://developer.apple.com/documentation/appkit/nsworkspace/icon(for:)
- [A2] Apple, `NSWorkspace.icon(forFile:)`: https://developer.apple.com/documentation/appkit/nsworkspace/icon(forfile:)
- [A3] Apple, SF Symbols: https://developer.apple.com/sf-symbols/
- [A4] Apple HIG, SF Symbols: https://developer.apple.com/design/human-interface-guidelines/sf-symbols
- [V1] VS Code, File Icon Theme guide: https://code.visualstudio.com/api/extension-guides/file-icon-theme
- [L1] https://github.com/catppuccin/vscode-icons/blob/main/LICENSE · [R1] https://github.com/catppuccin/vscode-icons (README, `src/defaults/fileIcons.ts`, `icons/{latte,frappe,macchiato,mocha,css-variables}`)
- [L2] https://github.com/material-extensions/vscode-material-icon-theme/blob/main/LICENSE · mapping `src/core/icons/fileIcons.ts`, `languageIcons.ts`
- [L3] https://github.com/vscode-icons/vscode-icons/blob/master/LICENSE · [R2] README "License" section of the same repo
- [L4] https://github.com/jesseweed/seti-ui/blob/master/LICENSE.md · [R3] README of the same repo
- [L5] https://github.com/devicons/devicon/blob/master/LICENSE · [R4] README of the same repo
- Counts and sizes: `GET https://api.github.com/repos/<owner>/<repo>/git/trees/<branch>?recursive=1`, 2026-10-17
- System-icon measurements: a local Swift script using `UTType(filenameExtension:)` + `NSWorkspace.shared.icon(for:)` with 32 px bitmap hashing, macOS 27.0.1, 2026-10-17
