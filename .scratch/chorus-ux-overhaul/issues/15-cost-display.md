# Cost display

Type: grilling
Status: resolved
Blocked by: 01, 19
Parent: ../map.md
Labels: wayfinder:grilling

## Question

What should the context/token panel show about cost? Decide: tokens only or money too; the currency (THB?) and where prices and exchange rates come from; how subscription accounts (no per-token price) are shown; and per turn, per session, and per sub session totals.

Input from Live updates while a turn runs: `session_usage` (schema v7) stores the last usage per session. The ACP adapter (codex.rs:924) keeps a USD cost from `usage_update.cost`. Per the test at codex.rs:1020, claude-agent-acp sends it and Codex does not, and non-USD costs are dropped.

## Answer

Perth accepted all five recommendations (2026-10-17).

1. **Money:**
   - Tokens always show. Money shows only when the bridge reports it, which today means claude-agent-acp's `usage_update.cost`, labelled **"มูลค่าเทียบ API"** because subscription users don't pay per token. Codex shows "ไม่มีข้อมูลราคา".
   - No price-table estimates. **This removes an existing path:** `ledger.rs:311` calls `local_router::calculate_cost` with a hardcoded per-model rate table (`local_router.rs:27-43`, e.g. opus 15/75 USD per 1M). `chorus_session_cost_usd` feeds that estimate to the panel at `ContentView.swift:2954`. Replace it with the bridge-reported cost from `session_usage`. The router's own use of `calculate_cost` (`local_router.rs:79, 99`) is a ranking heuristic, and Route presets' curated tier table supersedes it.
2. **Currency:**
   - Show USD first. Convert to THB with a rate the user types in Settings, with no network fetch, matching the release audit's zero-external-telemetry policy.
   - There is no default rate. Until one is set, show USD with a "ตั้งอัตราบาท" link.
3. **Totals:**
   - Per turn on hover, per session in the panel, and per sub session on its sidebar row.
   - The orchestration total (parent plus all children) shows on the parent row.
   - Daily and monthly totals stay in the fog.
4. **Quota:** show 5-hour and weekly quota bars (used % and reset time) where `limits.rs` can read them (Claude, Codex), above money.
5. **Panel order:** context meter → quota → session tokens in/out → money if any. It updates mid-turn through Live updates and is seeded from `session_usage` on reopen.
