# System monitor

Status: ready-for-agent
Blocked by: 01
Parent: ../spec.md
Decided in: [System monitor](../../chorus-ux-overhaul/issues/14-system-monitor.md) · [research 02](../../chorus-ux-overhaul/research/02-process-port-monitoring-apis.md) · [research 03 §4](../../chorus-ux-overhaul/research/03-security-delta-scan.md)

## What

- A new Rust module using libproc through `libc` (no new dependency) for listening sockets, phys_footprint, CPU (mach ticks converted by timebase), and the process tree.
- A Chorus pid registry for labelling.
- Service names come from the executable path, then the command line, then `/etc/services`. Never keep or log the environment from `KERN_PROCARGS2`, and mask token-like arguments.
- A separate window (`⌥⌘M`, in the Window menu) plus a RAM label in the sidebar footer.
- **Groups:**
  - Chorus: app, daemon, bridges, local model.
  - Services: `Port · Service · Process · PID · CPU · RAM · started by · uptime`, with a UDP toggle. Other users' processes are greyed out.
- **Refresh:** 1 s while visible, stopped while hidden.
- **Stop flow:**
  1. Re-check uid, start time and path, plus the system-path denylist.
  2. Confirm with the user.
  3. Send SIGTERM.
  4. After 3 s, an explicit "บังคับปิด" sends SIGKILL.
  5. `killpg` is used only for groups Chorus created.
  - No MCP tool for any of this.
- Disk usage by category and per project.
- Manual optimise actions, each with a size preview: blob GC, VACUUM (idle only), worktree prune, checkpoint prune (keeps `review_base`), unload model, clear caches, delete unused models.

## Done when

- A canary test opens a listener and checks the scan finds it.
- Unit tests cover the stop-guard rules.
- Check in the app: a local `redis-server` or `python3 -m http.server` appears, stops cleanly, and a SIGKILL needs the explicit click.
