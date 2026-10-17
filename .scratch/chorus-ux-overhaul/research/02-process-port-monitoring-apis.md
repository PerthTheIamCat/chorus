# Process and port monitoring on macOS (research for ticket 02)

Question: [../issues/02-process-port-monitoring-apis.md](../issues/02-process-port-monitoring-apis.md).
Researched 2026-10-17 on macOS 27.0.1 (26A434), arm64, as uid 501 (not root, `sudo -n` fails).
Measured with a throwaway C probe that only reads `libproc` (no process was signalled) and with
`lsof`, `netstat`, `top`, `ps`, `du`.

## Summary

- **Use `libproc` directly through the `libc` crate** (already a dependency of `chorus-daemon`).
  One sweep over every pid, with per-process task info, rusage and socket enumeration, took
  **2.7–4.7 ms for 591–607 pids**. That is cheap enough to poll every 1–2 s while the monitor is
  open. Only the socket structs need hand-written `#[repr(C)]` mirrors (about 80 lines).
- **Show memory as `ri_phys_footprint`** (from `proc_pid_rusage`), not RSS. It matches `top`'s
  MEM column and Activity Monitor. RSS was **8 MB vs 122 MB footprint** for the running Chorus app.
  `sysinfo::Process::memory()` returns RSS, so it would show the wrong number.
- **Without root you see only your own uid.** That is exactly the scope the map chose. 179 of 591
  pids (root/system daemons) return `EPERM` for task info, fds and rusage. Name, path, ppid and uid
  are still readable.
- **Stopping**: confirm → re-check identity (uid, start time, path) → `kill(SIGTERM)` →
  poll for `ESRCH` → `SIGKILL`. Use `killpg` for agent trees, which already run in their own
  process group (`crates/chorus-acp/src/lib.rs:186`).
- **"System" processes can share your uid** (ControlCenter on :5000/:7000, rapportd on :54631).
  A uid check is not enough. Also refuse anything whose executable sits under an Apple system path.

## 1. Candidate APIs compared

| | `libproc` via `libc` | `sysinfo` 0.39.6 | `listeners` 0.6.1 / `netstat2` 0.11.2 | `libproc` crate 0.14.11 | `lsof` / `netstat` subprocess |
|---|---|---|---|---|---|
| Listening ports + pid | yes (`PROC_PIDLISTFDS` + `PROC_PIDFDSOCKETINFO`) | **no** (no per-process sockets) | yes, same libproc calls | yes (`pidfdinfo`, `net_info.rs`) | yes, text parsing |
| CPU time | `pti_total_user/system` (mach ticks) | `cpu_usage()` % (does the delta) | no | yes | `ps` |
| Memory, footprint | `proc_pid_rusage` → `ri_phys_footprint` | **RSS only** (`pti_resident_size`) | no | yes (`pidrusage`) | `top -stats mem` |
| Tree | `pbi_ppid`, `pbi_pgid`, `proc_listchildpids` | `parent()`, `tasks()` | no | yes | `ps -o ppid,pgid` |
| Kill | `kill`/`killpg` | `kill_with()` = plain `kill(2)` | no | no | `kill` |
| New deps | none (libc 0.2.189 already in lockfile) | objc2 + IOKit (default features) | `byteorder` + others | **bindgen build-dep (needs libclang)** | none, but spawn + parse |
| Cost | ~3 ms full sweep | one sweep plus a `KERN_PROCARGS2` sysctl for each new pid | same sweep | same | `lsof -iTCP -sTCP:LISTEN` 23 ms, `netstat -anv` 12 ms |

Sources and evidence:

- `libproc.h` (macOS SDK, `usr/include/libproc.h`) declares `proc_listallpids`,
  `proc_listchildpids`, `proc_pidinfo`, `proc_pidfdinfo`, `proc_pidpath`, `proc_pid_rusage`.
  The header calls them "private interfaces … subject to change in future releases". They have
  shipped since 10.5 and `lsof`, `top` and `sysinfo` all use them, but we should keep a
  `lsof`-based self-test in mind (see Risks).
- `sys/proc_info.h` defines `PROC_PIDLISTFDS`=1, `PROC_PIDTBSDINFO`=3, `PROC_PIDTASKINFO`=4,
  `PROC_PIDT_SHORTBSDINFO`=13, `PROX_FDTYPE_SOCKET`=2, `PROC_PIDFDSOCKETINFO`=3,
  `SOCKINFO_IN`=1, `SOCKINFO_TCP`=2, `TSI_S_LISTEN`=1, and `struct socket_fdinfo`
  (`soi_kind`, `soi_proto.pri_tcp.tcpsi_state`, `tcpsi_ini.insi_lport`).
  **`insi_lport` is in network byte order.** The probe needed `ntohs`, and `netstat2` likewise
  uses `byteorder::NetworkEndian`.
- `libc` 0.2.190 (`src/unix/bsd/apple/mod.rs`) binds `proc_pidinfo`, `proc_pidfdinfo`,
  `proc_listallpids`, `proc_listchildpids`, `proc_pid_rusage`, `proc_pidpath`, `proc_fdinfo`,
  `proc_bsdshortinfo`, `rusage_info_v4`, `PROC_PIDLISTFDS` and `PROX_FDTYPE_SOCKET`. It does
  **not** bind `socket_fdinfo`, `in_sockinfo`, `tcp_sockinfo`, `PROC_PIDFDSOCKETINFO`,
  `SOCKINFO_TCP` or `TSI_S_LISTEN`. Those structs have to be mirrored by hand.
  `listeners/src/platform/macos/c_socket_fd_info.rs` (8.3 KB) is a working reference.
- `sysinfo` 0.39.6: `memory` is set from `task_info.pti_resident_size`
  (`src/unix/apple/macos/process.rs:447,796`). Kill is `kill(self.pid.0, c_signal)` (`:123-126`).
  CPU% needs two refreshes at least `MINIMUM_CPU_UPDATE_INTERVAL` = 200 ms apart
  (`src/unix/apple/system.rs:74`). Per-process disk I/O comes from `proc_pid_rusage`
  `ri_diskio_bytes*` (`process.rs:812-826`). Default features pull in IOKit and CoreFoundation
  bindings (`Cargo.toml` `[features] default`). The README says the default build is not
  App-Store-safe and suggests `apple-sandbox` for sandboxed apps. Chorus is not sandboxed: the
  installed app carries no entitlements (`codesign -d --entitlements -`), and
  `apps/macos/build.sh:70` explains why.
- `listeners` 0.6.1 `src/platform/macos/mod.rs::get_all()` loops over every pid → `SocketFd` →
  `proc_pidfdinfo`. That is the same mechanism as our probe, so it has the same visibility limits.
  It returns ports but no CPU/memory.
- `libproc` crate 0.14.11 `Cargo.toml`: `[build-dependencies.bindgen]` (runtime libclang). That
  is a heavier build for wrappers we can write in a few lines on top of `libc`.

## 2. Measured behaviour (non-root)

Probe results (three runs). Each run called `proc_listallpids`, then per pid: short BSD info,
task info, rusage v4, and fd list with socket info:

```
pids=607 mine=429 bsd ok/fail=607/0 task ok/eperm=429/178 rusage ok/fail=429/178 fds ok/eperm=429/178 tcp_listen=14 elapsed=4.7ms
pids=591 mine=412 bsd ok/fail=591/0 task ok/eperm=412/179 rusage ok/fail=412/179 fds ok/eperm=412/179 tcp_listen=14 elapsed=2.7ms
pid1 taskinfo r=0 errno=1 (EPERM)   pid1 listfds r=0 errno=1   pid1 path r=13 /sbin/launchd
```

- For other users' processes: short BSD info (name, uid, ppid, pgid, status) and
  `proc_pidpath` work. Task info, rusage and fd lists fail with `EPERM`. So we can *name* a root
  process but not see its sockets or memory.
- **The 14 TCP listeners found by the probe matched `lsof -iTCP -sTCP:LISTEN -nP` one for one**,
  including Chorus itself on `127.0.0.1:60738`. `lsof(8)` says its search is "restricted by the
  permissions of the lsof process", so it has the same scope.
- `netstat -anv -p tcp` listed **one extra listener: `cloudflared:34997` on 127.0.0.1:20241,
  owned by root.** netstat reads the kernel's socket table rather than per-process fds, so it can
  show *that a port is taken* even when the owner is another user. This is useful only for an
  "in use by another user" hint, never for kill. The `process:pid` column is not documented in
  `netstat(1)`, so treat it as best effort.
- UDP is noisy. Bound UDP sockets included mDNS 5353 ×8 in "Codex (Service)", port 0 sockets, and
  ephemeral client ports (macOS ephemeral range `net.inet.ip.portrange.first..last` =
  49152–65535). If UDP is shown at all, drop port 0 and ephemeral ports by default.
- CPU time units: `pti_total_user` is in **mach absolute-time ticks**, not ns, on Apple Silicon.
  The measured `34,205,515` ticks × (timebase 125/3) = 1.425 s, and `getrusage` reported
  1,425,229 µs. Convert with `mach_timebase_info`; on Intel the timebase is 1/1. `sysinfo` does
  the same (`macos/system.rs:83-104`). CPU% = Δ(user+system) × numer/denom ÷ Δwall-ns.
- Memory, footprint vs RSS, measured in the same instant:

  | process | `pti_resident_size` (RSS) | `ri_phys_footprint` | `top` MEM |
  |---|---|---|---|
  | Chorus 55901 | 8.2 MB | 121.9 MB | 122M |
  | zed 90668 | 12.6 MB | 364.1 MB | 364M |
  | Discord Helper 18664 | 30.6 MB | 526.1 MB | 526M |

  `footprint(1)` defines footprint as "the total of all Dirty memory", including swapped and
  compressed memory. It notes that its non-default `--vmObjectDirty` mode "may calculate a total
  footprint that does not match what is shown in other tools such as top(1) or Activity
  Monitor.app". That makes the default (pmap/phys_footprint) accounting the one those tools use.
  RSS undercounts badly because compressed pages are not resident. `footprint(1)` also says it
  "must be run as root when inspecting processes that are not owned by the current user", which
  is the same limit we hit.

## 3. Process tree: Chorus and the agents it spawned

- Agent bridges are spawned with `command.process_group(0)` and torn down with
  `killpg(pid, 9)` (`crates/chorus-acp/src/lib.rs:186,103-116,404-411`). So **pgid == the agent's
  pid** for everything the bridge starts, unless a descendant calls `setsid`/`setpgid` itself.
- Labelling rule, in order:
  1. pid is in Chorus's own registry of spawned agents → "agent";
  2. `pbi_pgid` equals a registered agent pid → "spawned by agent X";
  3. walk `pbi_ppid` up to Chorus's pid → "spawned by Chorus".
  Orphans are reparented to launchd (observed: Chorus.app itself has ppid 1), so (3) misses a dev
  server whose agent already exited. (2) still catches it while the pgid survives. A daemonising
  server (`setsid`) escapes both, so treat the label as a hint, not proof.
- `proc_listchildpids` gives direct children only. Build the tree from one `ppid` map per sweep
  instead of N calls.
- Today Chorus registers only a single `ACTIVE_PID` (`lib.rs:189`). Labelling several agents and
  sub sessions needs a set of (pid, start time, agent id) records. That is an implementation
  ticket, not decided here.

## 4. Naming a known service

1. **Executable name or path first.** Use `proc_pidpath` (works for any uid) or `pbsi_comm`
   (16 chars: probe shows `coujai-gateway-`). A small curated table maps `redis-server`,
   `postgres`, `mysqld`, `mongod`, `ollama`, `cloudflared`, `node`, `python` and so on.
2. **Then the argv** for generic runtimes (`node …/next dev`, `python -m http.server`). Read it
   with the `KERN_PROCARGS2` sysctl, as `sysinfo` does (`macos/process.rs:543-591`).
   **That buffer also contains the environment.** Parse argv only, never keep or log env (CLAUDE.md
   §11, 7.18: secrets).
3. **Last, `getservbyport(3)`** (`/etc/services`): it has `postgresql 5432`, `mysql 3306`,
   `http-alt 8080`. It has **no entry for 6379 (redis)**, and port-only guesses are wrong for any
   dev server on 3000/5000/8080. Show it as "usually X", below the exe match.
- Do not probe ports (connect + banner). That is network I/O on the poll path and talks to
  services the user did not ask us to touch.

## 5. Stopping a process safely

From `kill(2)`: a signal needs the sender's real or effective uid to match the target's, unless
the sender is root. So a non-root Chorus physically cannot signal other users' processes (EPERM).
The map's rule is enforced by the kernel, but the UI should still not offer the button.

Recommended sequence (no new deps; `libc::kill`/`killpg`):

1. Gate. Refuse if any of these hold:
   - uid ≠ `getuid()`;
   - pid ∈ {1, Chorus's own pid};
   - the executable path starts with `/System/`, `/usr/` (but not `/usr/local/`), `/bin/`,
     `/sbin/`, `/Library/Apple/`, or `/Applications/` entries that are Apple's.
   The ControlCenter, rapportd and sharingd processes found here all run as uid 501, own
   listening ports, and launchd restarts them anyway.
2. Confirm in the UI. Show name, pid, port, the label from §3, and footprint.
3. **Re-identify just before signalling.** Pids are reused (`kern.maxproc` = 2000 here). Compare
   `pbi_start_tvsec/usec` and the path against the values captured when the row was drawn. A
   mismatch means the process is gone, and Chorus must not kill whatever now holds that pid.
4. `kill(pid, SIGTERM)`. For a registered agent, use `killpg(pgid, SIGTERM)`.
5. Poll `kill(pid, 0)` every 100 ms for about 3 s. `ESRCH` means it is gone. A zombie (`pbsi_status`
   SZOMB) still answers `kill(pid,0)`, so treat SZOMB as gone, and reap it with `waitpid` if it is
   Chorus's own child (`Agent::drop` already calls `child.wait()`).
6. Still alive → `SIGKILL` (group for agents) → poll again → report the result. Never escalate
   without showing it.

`sysinfo::kill_and_wait` sends one signal and then waits; it has no TERM→KILL escalation, so it
saves nothing over the six lines above.

## 6. Polling cost

- Full sweep ≈ 3 ms CPU at ~600 pids. If the scope is limited to our uid,
  `proc_listpids(PROC_UID_ONLY, uid, …)` (`PROC_UID_ONLY`=4 in `proc_info.h`) drops the ~180
  EPERM calls up front.
- Suggested cadence: 1 s while the monitor pane is visible, 0 when hidden. For agent rows only
  (the sidebar status), 5 s is enough. CPU% needs two samples, so keep the previous tick totals
  per (pid, start time).
- Run it on its own thread with no SQLite access. It does not touch the engine handle, so CLAUDE.md
  7.7's single-queue rule does not apply. Do not put it on the turn queue.
- Subprocess alternatives cost 12–23 ms each plus parsing, and `lsof` output is not a stable
  format. Use them only as a test oracle.

## 7. Measuring Chorus's own disk usage

Where the bytes live (from code):

| what | path | source |
|---|---|---|
| ledger + blobs + FTS + summary | `<workspace>/.chorus/store.db` (+ `-wal`, `-shm`) | `ContentView.swift:1579`; blobs are a SQLite table, not files |
| shadow checkpoint repo | `<workspace>/.chorus/ckpt` | `checkpoint.rs`, CLAUDE.md 7.3 |
| worktrees | `<workspace>/.chorus/worktrees/<role>-<id>` | `worktrees.rs:33-36` |
| local models | `~/Library/Application Support/Chorus/models/` | `model_manager.rs:81-90` |
| per-project memory | `~/Library/Application Support/Chorus/projects/<uuid>/memory.db` | `Projects.swift:53` |
| model list cache | `~/.cache/chorus/models-<agent>.json` | `agents.rs:66` |

Measured here: this repo's `.chorus` is 17.2 MB allocated (`du -sk`) vs 19.0 MB apparent
(`du -skA`). `store.db` is 12.1 MB: 3097 pages × 4096 B, freelist 0. `ckpt` is 5.9 MB.
`~/Library/Application Support/Chorus` is 506 MB, mostly models. `du` of `.chorus` took 26 ms.

- Directory walks: sum `st_blocks × 512` for "on disk" (what `du` reports; `du(1)` says `-A` gives
  apparent size "when operating on compressed volumes or sparse files"). Both can be shown.
  Count each (dev, ino) once so hard links are not double-counted.
- Per-table breakdown of `store.db`: use `SELECT name, SUM(pgsize) FROM dbstat GROUP BY name`.
  The bundled `libsqlite3-sys` 0.30.1 compiles with `-DSQLITE_ENABLE_DBSTAT_VTAB`
  (`build.rs:126`), so this works through rusqlite in-app. The system `sqlite3` CLI does not have
  it ("no such table: dbstat"). `PRAGMA freelist_count × page_size` shows what `VACUUM` would
  reclaim.
- Worktrees are full checkouts. Their size is real disk use, not shared, so they are the likely
  big item to offer clean-up for.

## Recommendation

1. Add a `monitor` module in Rust (core or daemon; not Swift, CLAUDE.md 2.3) built on
   **`libc` + about 80 lines of hand-mirrored `socket_fdinfo` structs**. No new crate.
   `sysinfo` is rejected because it reports RSS, has no sockets and brings IOKit deps.
   `listeners`/`netstat2` are rejected because they cover ports only, so we would still write the
   rest. The `libproc` crate is rejected for its bindgen build-dep.
2. Report memory as `ri_phys_footprint` and label it as Activity Monitor's number. CPU% comes
   from mach ticks scaled by the timebase.
3. Scope: own-uid processes via `PROC_UID_ONLY`, TCP LISTEN by default, UDP behind a toggle with
   port-0 and ephemeral ports hidden. Optionally show "port held by another user" from the
   kernel table (`netstat -anv`-style) with no actions.
4. Kill: gate → confirm → re-identify by start time → TERM → poll → KILL. `killpg` for agents.
   Add an Apple-system-path denylist on top of the uid check.
5. Disk: a walk of the six locations above plus `dbstat` for `store.db`, computed on demand (the
   pane opening), not on the 1 s tick.

## Risks and open points

- `libproc` is officially "private … subject to change". Mitigation: one integration test that
  binds a TCP listener in-process and asserts the sweep finds it with the right pid and port. That
  catches struct-layout or byte-order breakage on a new macOS.
- Agent labels miss `setsid` daemons (§3). A stronger link needs a registry of spawned pids plus
  start times. That belongs in the sub-sessions ticket.
- Windows (WinUI later) needs `GetExtendedTcpTable` + `PROCESS_MEMORY_COUNTERS`. Keep the
  `monitor` API platform-neutral (rows of pid/name/port/footprint/label) so only the backend
  changes.
- Not verified: behaviour inside App Sandbox (Chorus is unsandboxed today), and Intel Macs
  (timebase 1/1; we use the same code path, but no machine was available to test).
