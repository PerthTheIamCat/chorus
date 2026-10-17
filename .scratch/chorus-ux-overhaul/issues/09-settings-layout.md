# Settings layout

Type: prototype
Status: resolved
Blocked by: 01
Parent: ../map.md
Labels: wayfinder:prototype

## Question

How should Settings be split into categories, with short labels and an (i) hover tooltip for longer explanations? Show the category list and two or three panes, including where orchestra mode and route presets are set.

Prototype: ./prototypes/09-settings-layout.html (variants A/B/C, panes routing/orchestra/safety)

## Answer

Perth picked **variant A, a System Settings-style sidebar** (2026-10-17). See [prototype](../prototypes/09-settings-layout.html) `?variant=A`.

- **13 categories in 4 groups:**
  - **ใช้งาน:** ทั่วไป · บัญชี · การส่งงาน · Orchestra
  - **Agent:** โมเดล · สิทธิ์และความปลอดภัย · เครื่องมือเสริม (MCP) · Skills
  - **ข้อมูล:** Review และ checkpoint · การใช้งานและค่าใช้จ่าย · พื้นที่และประสิทธิภาพ
  - **ระบบ:** Mobile Companion · เกี่ยวกับ
- **Each pane:** a title plus a one-line subtitle, then grouped rows. Each row has a short label, its control on the right, and an **(i)** hover/focus tooltip for any longer explanation. No pane needs to scroll past one screen.
- **Where things live:**
  - Mode and route preset defaults, plus failover order: "การส่งงาน".
  - Orchestrator and backup, caps, ask-before-plan, auto-merge, `.env` copy: "Orchestra".
  - Default permission, trusted projects (post-turn verify/hooks), Spotlight, on-device dictation: "สิทธิ์และความปลอดภัย".
  - THB rate: "การใช้งานและค่าใช้จ่าย".
  - Catppuccin licence credit: "เกี่ยวกับ".
- **Copy pass:** the Thai/English copy pass from Shell polish triage happens with this rebuild. Remove stale model ids and the unread orchestration keys.
