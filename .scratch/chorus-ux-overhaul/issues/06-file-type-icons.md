# File-type icon sources

Type: research
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:research

## Question

Where can a macOS app get accurate per-file-type icons (`.rs`, `.md`, `.swift`, `.ts`, `.json`, `.toml`, lockfiles, Dockerfile…)? Compare system icons (`NSWorkspace.icon(for: UTType)`), SF Symbols, and bundled icon sets (Material Icon Theme, vscode-icons, Seti, Catppuccin icons, Devicons): licence, coverage, file size, light/dark, and how to map extensions and special file names. Recommend one.

Research: ./research/06-file-type-icons.md

## Answer

Use a curated subset of Catppuccin Icons (MIT, about 60–100 icons; Latte for light mode, Mocha for dark). Keep SF Symbols for folders and unknown files. Don't use `NSWorkspace` icons: on macOS `.ts` resolves to a video type, and `.rs`, `.ts` and `.toml` all get the same icon. Match the way VS Code does: file name first, then the longest extension, case-insensitive. Rust looks up the `icon` key for each tree node and Swift loads the bundled SVG without template tinting. Ship the licence plus an About credit. Not yet checked: how the icons look at 13 pt in the real sidebar. Detail: [research](../research/06-file-type-icons.md).
