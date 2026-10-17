# Workbench layout v2

Status: ready-for-agent
Blocked by: 03
Parent: ../spec.md
Decided in: [Workbench layout v2](../../chorus-ux-overhaul/issues/07-workbench-layout-v2.md) · prototype `../../chorus-ux-overhaul/prototypes/07-workbench-layout-v2.html?variant=A`

## What

- Four resizable columns: Sessions (230–260 px) │ File tree (210–240 px) │ File viewer (only while a file is open) │ Chat (min 380 px).
- Below 1200 px: the tree becomes an icon rail, and an open file temporarily replaces chat.
- Remove the Center Stage thread/code split and the "agent worktrees" explorer list.
- The Inspector stays a toggled right panel (`⌥⌘I`). Widths persist (the existing `chorus.*Width` keys).
- The viewer shows a breadcrumb, a read-only badge and a close button, using the existing `highlight.rs` spans.
- The ● agent-focus mark on files comes from Rust (`chorus_show_file` / the last tool path). Confine the path to the workspace (security D3).

## Done when

- Check in the app at 1024, 1440 and 1800 px wide: no clipped columns, resizing persists across relaunch, opening and closing a file at 1024 px swaps chat correctly, and light and dark mode both look right.
