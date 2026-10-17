# Process and port monitoring on macOS

Type: research
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:research

## Question

How can Rust on macOS, without root, list listening TCP/UDP ports with their owning process, read CPU and memory for each process (and a process tree such as Chorus and the agents it spawned), find a known service's name (redis, postgres…), and stop a process safely? Compare libproc/`proc_pidinfo`, the `sysinfo` crate, `lsof`/`netstat` subprocesses, and what Activity Monitor shows (footprint vs RSS). Cover the cost of polling, the limits on other users' processes, and how to measure Chorus's own disk usage (ledger, blobs, worktrees, models, caches).

Research: ./research/02-process-port-monitoring-apis.md

## Answer

- **How to read processes:** `libproc` through the existing `libc` crate, plus about 80 lines of hand-written socket structs. One pass over about 600 pids takes 3–5 ms, so polling every 1 s on its own thread is fine while the monitor is open.
- **Memory:** show `ri_phys_footprint`. RSS is wrong (8 MB vs 122 MB for Chorus).
- **CPU:** times come in mach ticks, which need the timebase conversion.
- **Other users' processes:** about 30% give `EPERM`. Name, path, parent and uid still read, but CPU, memory and sockets don't.
- **Labels:**
  - Agent-spawned: a Chorus pid registry, then the process group, then the parent chain. This is a hint only.
  - Service name: executable path, then the command line (never keep or log its environment), then `/etc/services`.
- **Stopping:** in order:
  1. Check uid and the denylist of Apple system paths.
  2. Confirm in the UI.
  3. Re-check start time and path, since pids get reused.
  4. Send SIGTERM, then escalate to SIGKILL. Use `killpg` for agents.
- **Disk:** walk the six Chorus paths by `st_blocks`. Use `dbstat` for a per-table size of `store.db`.
- **Rejected:** `sysinfo`, `listeners`/`netstat2`, the `libproc` crate.
- **Risk:** Apple calls `libproc` private, so it needs a canary test.

Detail: [research](../research/02-process-port-monitoring-apis.md).
