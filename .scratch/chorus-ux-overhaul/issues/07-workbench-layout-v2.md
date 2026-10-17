# Workbench layout v2

Type: prototype
Status: resolved
Blocked by: 01, 06
Parent: ../map.md
Labels: wayfinder:prototype

## Question

How should the window look and resize when the file viewer sits on the left next to the file tree, files have type-specific icons, and sub sessions appear under their session in the sidebar (`session 1` / `|-- reviewer`)? Show narrow, medium, and wide windows. Say which parts of harness-roadmap ticket 13 this replaces.

Prototype: ./prototypes/07-workbench-layout-v2.html (variants A/B/C, ?variant=&w=)

## Answer

Perth picked **variant A, four columns** (2026-10-17). See [prototype](../prototypes/07-workbench-layout-v2.html) `?variant=A`.

- **Columns, left to right:** Sessions sidebar (230–260 px, sub sessions nested under their parent) │ File tree (210–240 px, Catppuccin file-type icons, git badges, ● on the file the agent is looking at) │ File viewer (opens next to the tree, only while a file is open; read-only, Xcode-style highlighting, breadcrumb, ✕ to close) │ Chat (min 380 px).
- **Narrow windows (< 1200 px):** the file tree shrinks to an icon rail, and an open file temporarily takes the chat's place. Closing the file brings chat back.
- **Wide windows:** columns grow toward their upper widths, and viewer and chat share the remaining space about 1.2 : 1.
- **Inspector** (Context/Changes/Memory/Checkpoints) stays as a toggled right panel (`⌥⌘I`). Monitor opens in its own window (`⌥⌘M`).
- **This replaces harness-roadmap ticket 13:** no Center Stage split between thread and code viewer, and no separate "Workspace Explorer" with an agent-worktrees list (Sub sessions removed that list).
- Not decided here: the visual styling of the real SwiftUI build. The mock only fixes structure and responsive behaviour.
