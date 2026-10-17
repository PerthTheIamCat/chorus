# Cost display

Status: ready-for-agent
Blocked by: 02, 03
Parent: ../spec.md
Decided in: [Cost display](../../chorus-ux-overhaul/issues/15-cost-display.md)

## What

- Remove the price-table estimate: `ledger.rs:311` calls `local_router::calculate_cost`, and `chorus_session_cost_usd` feeds the panel.
- Show money only from bridge-reported `usage_update.cost` (claude-agent-acp), labelled "มูลค่าเทียบ API". Codex shows "ไม่มีข้อมูลราคา".
- **Panel order:** context meter → 5-hour and weekly quota bars (`limits.rs`) → session tokens in/out → money.
- **THB:** converted with a user-set rate, no network. With no rate set, show USD plus a "ตั้งอัตราบาท" link.
- **Totals:** per turn (hover), per session, per sub session, and per orchestration on the parent row.

## Done when

- Tests: cost is summed from `session_usage`, and no estimate is produced for a bridge without cost.
- Check in the app: the panel updates mid-turn and survives a reopen.
