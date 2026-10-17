# Security delta since the 2026-10-12 release audit

Date: 2026-10-17. Static review only. No source edits, no builds, no install scripts, no commits. No `.env`, credential, Keychain or private database was read. This is a bounded source review, not a pass/fail certificate. Format follows `../../chorus-release-audit/` (path:line, reachable trigger, impact, confidence, verification).

## 1. What "since the audit" means here

- **Baseline commit:** `5af2ac2` (2026-10-05 22:08) is the last commit dated on or before 2026-10-12. The audit itself reviewed the *uncommitted* tree on 2026-10-12 ("Cover all current code, including uncommitted files", `../../chorus-release-audit/map.md`). That tree was committed later as part of `61e1c87` (2026-10-14 22:21, 245 files, +34,427). So `git diff 5af2ac2..61e1c87` mixes audited and unaudited code.
- **How the two were separated:** working-tree mtimes, compared with the last audit fix documents (`git-sandbox-fixes.md` / `digest-preview-fixes.md`, 2026-10-12 16:50).
  - **Changed after the audit** (strict delta): `crates/chorus-daemon/src/client.rs`, `crates/chorus-daemon/src/pairing.rs`, `crates/chorus-ffi/src/lib.rs`, `crates/chorus-ffi/include/chorus.h`, `apps/macos/Sources/Chorus/RemotePairingView.swift` (all 2026-10-14 19:57–20:00; this is audit ticket 10's "Remote pairing integration"), plus `0c1bfc9` (`build.sh`, `chorus-ffi/Cargo.toml`, `.gitignore`) and `apps/macos/Package.swift` (adds the Vision and CoreSpotlight frameworks).
  - **Present at audit time but never named in any audit evidence:** `agent_control.rs`, the MCP control-surface tools in `mcp.rs`, `roles.rs`, `verify_loop.rs`, `worktrees.rs`, `hooks.rs`, `limits.rs` (Antigravity quota), `antigravity_bridge.rs`, `bridge_for` changes in `codex.rs`, and the Swift files `OrchestratorView`, `CodeViewerView`, `TerminalDrawerView`, `KeychainManager`, `SpotlightIndexer`, `VoiceDictationManager`. These are in `61e1c87` and in scope for this ticket. I reviewed them wherever they touch a trust boundary: spawning processes, filesystem paths, the network, the ledger, or what the agent can control.
- **Uncommitted changes:** `CONTEXT.md` (glossary only) and `.scratch/`. No uncommitted code.
- **`0c1bfc9`:** no security effect. It splits the cargo invocations and drops the `cdylib` crate type, which removes one exported artifact.

Severity uses the audit's scale: **P1** = a path to unsandboxed code execution, writes, or a bypass of the permission/sandbox contract. **P2** = a confirmed control defect with a realistic trigger. **P3** = hardening, privacy, or evidence-quality issues. **Dormant** = code that nothing in the shipped app or CLI calls today, but that the orchestra mode in this map will make reachable.

## 2. Findings

### P1-1: automatic `cargo check` after every turn runs outside Seatbelt (sandbox escape)

| | |
|---|---|
| Source | `apps/macos/Sources/Chorus/ContentView.swift:993-1000` calls `engine.verifyWorkspace(ws:)` whenever `engine.busy` becomes false (every finished turn). There is no setting or confirmation. `Engine.swift:2250-2256` → `chorus_verify_workspace_json` (`crates/chorus-ffi/src/lib.rs:2284`) → `VerifyLoop::verify_workspace` (`crates/chorus-core/src/verify_loop.rs:65-94`) runs `cargo check` in the workspace whenever `Cargo.toml` exists. `biome`, `ruff` and `swiftlint` run for other project types (`:139-203`). |
| Reachable trigger | (a) In Workspace/"ให้ทำเลย" mode, or "ถามก่อน" with an agent that never asks (CLAUDE.md 7.10), a sandboxed agent writes `build.rs`, a proc-macro, or `.cargo/config.toml` with `build.rustc-wrapper = "./x.sh"` inside the workspace. Seatbelt allows ordinary workspace writes. When the turn ends, Chorus itself runs `cargo check`, which compiles and runs build scripts and honours `rustc-wrapper`. (b) Opening a hostile Rust repo and sending any one turn runs its `build.rs` with no agent involved. |
| Impact | The agent gets arbitrary code execution as the user with no Seatbelt. That is exactly the escape the `.git/hooks` / `.git/config` carve-outs exist to stop ("the user's `git` runs it later outside the sandbox", CLAUDE.md 7.20). Here Chorus runs it immediately. It also bypasses `revert_if_touched`: the payload runs after the turn. |
| Confidence | Mechanism: high. Cargo docs say build scripts are compiled and run before the package builds, and `build.rustc-wrapper` wraps every rustc call, including for `check`. Reachability: medium. `Command::new("cargo")` resolves through the app's own `PATH`. A Finder/`open`-launched app gets launchd's minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), and nothing in the app adds `~/.cargo/bin` for this call (`session.rs:1336-1347` changes PATH for agent bridges only). So it is reachable when cargo is on that PATH, when the app is started from a shell (`swift run`, Xcode, `.build/.../Chorus`), or in any future change that imports the login-shell PATH. |
| Verification | Static trace of caller → FFI → command. Not executed: the static-only rule applies, and running it would build untrusted code. |
| Sources | [Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html), [Cargo config `build.rustc-wrapper`](https://doc.rust-lang.org/cargo/reference/config.html#buildrustc-wrapper) |

### P2-1: a role name from repo files escapes `.chorus/agents/` on save and delete

| | |
|---|---|
| Source | `crates/chorus-core/src/roles.rs:83` takes `name:` from the frontmatter of any `.chorus/agents/*.md` without validation. `roles.rs:182-187` `save_to_dir` writes `dir/.chorus/agents/{name}.md`. `crates/chorus-ffi/src/lib.rs:2147-2162` `chorus_delete_role` removes `cwd/.chorus/agents/{name}.md`. Swift passes the loaded name straight through: `OrchestratorView.swift:163` (delete) and `:652-657` (`quickAssignModel` re-saves the role under its loaded name). |
| Reachable trigger | A cloned repo ships `.chorus/agents/x.md` with `name: ../../../.claude/CLAUDE` or with an absolute name such as `/Users/<u>/.claude/CLAUDE`. `Path::join` with an absolute component *replaces* the base. The user opens the orchestrator pane and changes that role's model, or deletes the role. |
| Impact | Writes attacker-shaped Markdown (the role's prompt/body) to any `*.md` path the user can write, or deletes one. Overwriting `~/.claude/CLAUDE.md`, another project's `AGENTS.md`, or a skill `SKILL.md` plants a persistent prompt injection outside Chorus. The `.md` suffix limits the damage to instruction files, but those are exactly the files agents trust. |
| Confidence | High for the path construction. The Rust `Path::join` semantics are documented. Needs a click from the user on the hostile role, whose name is visible but looks plausible. |
| Verification | Static trace, plus Rust std docs. No file was written. MCP `chorus_synthesize_role` slugifies its hint (`roles.rs:340-341`, `:432-441`), so the agent-supplied path is safe today. Only names loaded from files (and the Swift editor's free text) are not. |
| Sources | [`Path::join`](https://doc.rust-lang.org/std/path/struct.Path.html#method.join) |

### P2-2: remote pairing: revoke and the device list fail open in the UI

| | |
|---|---|
| Source | `RemotePairingView.swift:270-279` removes the device row whatever `chorus_pairing_revoke_json` returned. `crates/chorus-ffi/src/lib.rs:2392-2412`: if the daemon request fails for any reason (timeout, non-200, parse), it falls back to an in-process `PairingManager` that never paired anything, and returns `ok:false`. That result is ignored. `lib.rs:2352-2387`: when the daemon socket exists but the request fails, the device list falls back to the same empty in-process manager and reports `ok:true, devices:[]`. |
| Reachable trigger | The daemon is busy or slow (10 s client timeout, `client.rs:34`), restarting, or replying with a non-200 while the user revokes a lost phone, or opens the pairing sheet. |
| Impact | The user is told a device is revoked, or that no devices are paired, while its `cpt_` bearer token stays valid on the daemon. A paired device can list and resolve permission interactions (`http.rs:250-272` allows every non-admin TCP route to paired tokens). This is a fail-open on the one control a user reaches for after losing a device. |
| Confidence | High (straight-line code). |
| Verification | Static. The audit's `ffi_pairing_lifecycle` test only covers the in-process path, so it cannot catch this. |

### P2-3: the MCP control surface gives agent text user authority and crosses session boundaries

| | |
|---|---|
| Source | `crates/chorus-core/src/mcp.rs:279-290` advertises 10 new tools to every agent. `session.rs` also tells every agent about them in the turn preamble. Problems: `agent_control.rs:266-272` (`send_message`) inserts a raw `event` row with `kind=0` (Msg), `actor=1`, `t=0`, `parent=NULL` into the child session. `mcp.rs:445-461` (`spawn_agent`) inserts `kind=1` with `actor = SELECT id FROM actor LIMIT 1`. In a store created by `Session::new`, actor 1 is `"user"` (`session.rs:351`, `:382` intern `user` first). `mcp.rs:491-494` (`send_message`), `:551-553` (`cancel_agent`) and `:508`/`:536` (`wait`/`collect` with explicit ids) take any `agent_id` without checking it belongs to `session_scope` (`agent_control.rs:208-215`, `:249-251`, `:346-360`, `:370-373`). `mcp.rs:586/605/621` fall back to session `1` when the server is unscoped (`session.rs:519`, the project-memory server). |
| Reachable trigger | An agent in session A, or prompt-injected text it read, calls `chorus_send_message` with a guessed small integer id. |
| Impact | (a) Agent-written text appears in the ledger as a **user** message. That is the single source of truth: task state takes "goal = the user's first message" (CLAUDE.md §6), handoff briefs and agent blame read it, so the next agent in that child session receives the injection with user authority. (b) Agents can cancel or read other sessions' sub-agents. (c) Raw inserts skip `ledger::append`: no parent link (`history()` walks parents), no blob dedup, `t=0`. That breaks rule 2.2 integrity and bench invariant reasoning. Today no process runs from these rows (see D5), so the effect is ledger and UI integrity. Orchestra mode turns it into control over running agents. |
| Confidence | Medium-high. Actor-1-is-user holds for stores created by `Session::new`. It was not checked against a real database (private DB, not read). |
| Verification | Static. |

### P2-4: the MCP server has no authentication on loopback; any local user can reach it (pre-existing, made worse)

| | |
|---|---|
| Source | `mcp.rs:51-55` binds `127.0.0.1:0`. `mcp.rs:96` advertises `headers: []`. The only check is that the request has no `Origin` (`mcp.rs:159-186`). The baseline `5af2ac2` already exposed `memory_propose` and the session-scoped `attachment_read` this way. The delta adds spawn/send/cancel/checklist/jump/review/notify. |
| Reachable trigger | Any local process, **including other macOS user accounts** (loopback TCP is not per-user), scans the ephemeral range and POSTs JSON-RPC. |
| Impact | Reads the user's attached files for the live session (`attachment_read`). Queues memory proposals. Takes all the P2-3 actions. Posts notifications (P3-6). The daemon was fixed for exactly this class (bearer tokens on TCP, `fixes.md` P1). The per-session MCP server never was. |
| Confidence | High for missing auth. Medium for the multi-user threat model, which matters on shared Macs. |
| Verification | Static, compared against the baseline. The MCP spec's security guidance asks local servers to authenticate as well as bind to loopback. |
| Sources | [MCP transports: security warning](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports), [MCP security best practices](https://modelcontextprotocol.io/specification/2025-06-18/basic/security_best_practices) |

### P2-5: `bridge_for` runs `target/*/antigravity-acp` relative to the process cwd

| | |
|---|---|
| Source | `crates/chorus-acp/src/codex.rs:94-115`. If `antigravity-acp` is not on PATH or next to the executable, it tries `target/debug/…`, `target/release/…`, `../../target/debug/…` and `../../target/release/…` relative to the **process** working directory, not the Chorus install. |
| Reachable trigger | The user runs the `chorus` CLI from inside a cloned repo that commits `target/debug/antigravity-acp`, and picks an Antigravity agent, with no bridge installed. The app bundle normally finds its bundled copy first (`current_exe` dir, `:97-103`). |
| Impact | The repo's binary runs as the agent bridge. Seatbelt wraps it, so writes stay confined, but reads and network are open: it can read and exfiltrate `~/.ssh`, `~/.codex/auth.json` and so on. |
| Confidence | Medium (needs a specific CLI setup). |
| Verification | Static. |

### P3 findings

| ID | Source | Trigger → impact | Confidence / verification |
|---|---|---|---|
| P3-1 | `crates/chorus-cli/src/limits.rs:601-667` | Each quota poll scrapes `ps -A` (all users' processes) for `language_server --csrf_token`, takes the token, and replays it with `curl -k` (TLS checks off) to every loopback port that pid listens on. The result is cached to `~/.chorus/cache`. This harvests another product's credential, against the "never read credentials" rule (CLAUDE.md §6 auth detection, §11). Another user's look-alike process can feed fake quota data into routing and failover. | High / static |
| P3-2 | `crates/chorus-acp/src/bin/antigravity_bridge.rs:100-101` | The whole assembled prompt (context, memory, attachment text) goes to `agy` as an argv string, so any local user can read it through `ps`. Very long prompts also fail against `ARG_MAX`. | High / static |
| P3-3 | `RemotePairingView.swift:208`; `crates/chorus-ffi/src/lib.rs:2311-2341` | Swift hardcodes `http://<tailscale_ip>:24678`. That brings back the fixed-port problem the audit fixed ("advertise the bound endpoint", `fixes.md`) and puts endpoint logic in Swift (rule 2.3). When the daemon is down, the FFI makes a QR for an in-process manager that serves nothing. Another local user can bind `*:24678` and the phone will pair with them (spoofed approval feed). There is no server identity in the QR, so the phone cannot authenticate the host. | Medium / static |
| P3-4 | `crates/chorus-daemon/src/pairing.rs:113`, `http.rs:527-544` | `pairing_token != input` is not constant-time, and `/api/pairing/complete` (public on TCP) has no attempt budget. With 256-bit tokens and a 5-minute TTL this is a candidate, not an exploit. The audit checklist asked for an attempt budget. | Low / static |
| P3-5 | `crates/chorus-daemon/src/client.rs:129-133` | `resize(content_length)` trusts the server's header, so whatever answers on `~/.chorus/engine.sock` can make the app allocate without bound. Same-user threat only. The daemon side caps this, the client does not. | High / static |
| P3-6 | `mcp.rs:645-660` → `notify.rs:96-128` | An agent can post system notifications with any title and body (for example "macOS: re-enter your password"). The AppleScript escaping is correct (`notify.rs:89-94`), so this is spoofing, not injection. | High / static |
| P3-7 | `SpotlightIndexer.swift:17-25` (called from `Engine.swift:1487`); `VoiceDictationManager.swift:35`; `apps/macos/build.sh:55-74` | Each session's goal (the user's first message, which may contain pasted secrets) goes into the system Spotlight index, outside the app's data. Dictation does not set `requiresOnDeviceRecognition`, so audio may go to Apple. Both conflict with the 2026-10-14 policy "Zero external telemetry" (`issues/06-release-policy.md`). The generated Info.plist has no `NSMicrophoneUsageDescription` / `NSSpeechRecognitionUsageDescription`, so TCC terminates the app on first use. `LSMinimumSystemVersion` is 13.0 against the agreed 14.0. | High / static |
| P3-8 | `../../chorus-release-audit/issues/10-remaining-release-coverage.md` §4, `issues/05` Gate 2 | Evidence quality: it says "`KeychainManager.swift` is wired for secure token storage", but `KeychainManager` has **no callers** (`grep` finds only its own file). Ticket 10's other answers (pairing "verified", "220+ tests") rest on the in-process path only (see P2-2). Gate 2 should be reopened for credential storage and pairing. | High / static |

## 3. Dormant code that orchestra mode will make reachable

None of these has a caller in the app or CLI today (checked with `grep` over `apps/` and `crates/*/src`). They are listed because tickets 10, 11 and 13 will likely wire them up.

| ID | Source | What goes wrong once it is wired |
|---|---|---|
| D1 | `crates/chorus-core/src/worktrees.rs:177-209`, `:36` | `.worktreeinclude` is read from the repo and each line goes through `root.join(pattern)` / `target.join(pattern)` with no confinement (`..`, absolute paths, symlinks followed by `fs::copy`). `.env*` is copied into every worktree by default. The role name is interpolated into the worktree path and the branch name (same traversal as P2-1). `rand_u32` is time-based. `merge_worktree` runs `git commit` for the user. |
| D2 | `crates/chorus-core/src/hooks.rs:155-245` | Runs `.chorus/hooks/pre-dispatch*` / `post-turn*` from the repo, with no sandbox and no consent. Cloning a repo becomes code execution. `truncate_hook_output` (`:176-188`) slices at byte offsets and panics on Thai/UTF-8 output over 64 KiB. |
| D3 | `agent_control.rs:476-495`; `CodeViewerView.swift:219-226`, `:244-259` | `chorus_jump_to_file` stores any agent-supplied path. The viewer reads `cwd + path` without canonicalising or confining it, so an agent could steer the viewer to `../../.ssh/id_rsa`. The "open in editor" fallback `NSWorkspace.open` would *launch* a `.app`/`.command` file. Today the fallback only fires if `/usr/bin/env` fails. |
| D4 | `agent_control.rs:190-192`, `worktrees.rs:33` vs `crates/chorus-acp/src/sandbox.rs` `.chorus` carve-out | Worktrees live under `.chorus/worktrees/`, which the Workspace profile denies writes to (to protect the shadow repo). Either agents in worktrees can't write, or someone relaxes the carve-out and agents can tamper with `.chorus/ckpt`. This needs a decision before orchestra mode. |
| D5 | `agent_control.rs:169-205`, `:285-324` | `spawn_agent` only writes rows marked `running`. Nothing calls `complete_agent`, and `wait_agent` ignores `timeout_secs`. This matches ticket 10's audit note: orchestra mode must not treat these rows as proof that an agent ran or stopped. |

## 4. Requirements for the new surfaces in this map

These go into the spec at ticket 16 (triage). Each one is a testable rule, not a recommendation to add dependencies.

### Killing processes (system monitor, ticket 14; research 02)
1. Only signal a pid that passes all of these at click time *and again just before `kill`*: same uid as Chorus, start time and executable path unchanged since listing (pid reuse), not under the Apple system path denylist, not pid 1 or launchd's children of other sessions, not Chorus or its daemon unless the user takes an explicit "quit Chorus" path.
2. Confirm in the UI, showing name, pid, path, and whether the process is agent-spawned. Then SIGTERM, wait, then SIGKILL. Use `killpg` only for process groups Chorus created (from the pid registry), never for groups inferred from parent chains.
3. Labels like "agent-spawned" are hints. A kill decision must not rely only on a label the agent could influence (argv, process name).
4. Monitor output never shows or stores environment variables, and shows command lines with token-like arguments masked (P3-1 shows `--csrf_token` sitting in argv). Command lines are never sent to a model.
5. This is a Rust core action with a typed request; Swift only confirms (rule 2.3). Agents get no MCP tool to kill processes.

### Orchestra mode spawning agents (tickets 10, 11; harness 11, 12)
1. Fix P2-3 and P2-4 first. Every MCP call is authenticated with a per-session secret, delivered in the ACP `mcpServers[].headers` Chorus already controls. The session scope comes from that credential, never from arguments. Every `agent_id` is checked to belong to the caller's session tree.
2. Ledger writes from the control surface go through `ledger::append`, with the orchestrator agent's actor. They never use `actor=1`/`user`, never a raw `INSERT`, and keep the parent link. Messages from the orchestrator to a child are labelled as agent-authored in the child's context and in task state.
3. Children inherit a sandbox profile and permission policy that is **no wider than the parent's**. A role's `tools`/`write_mode` can narrow the policy but never widen it. Roles from repo files are untrusted: validate names to `[a-z0-9-]{1,64}` on load (closes P2-1 and D1's path issue), and show role prompts and changes to the user before first use (the map already says new roles are shown).
4. A hard concurrency cap and a spend/time cap per orchestration, enforced in Rust (not in the prompt). Cancelling the orchestrator cancels and reaps its children's process groups (reuse `db2ea2e` reaping).
5. Worktree rules: resolve D4 before shipping. `.worktreeinclude` must be confined to repo-relative paths with no `..`, no absolute paths, and no symlinks. Copying `.env*` must be opt-in and shown. Merging back needs explicit user approval, or the `autoMergeWorktrees` setting with a diff review. Never commit for the user without it.
6. Nothing Chorus runs *after* an agent turn may run outside Seatbelt on files the agent could have written (fix P1-1). Verification, hooks (D2) and linters run inside the same sandbox profile as the turn, or only after the user approves. Opening a repo never runs anything.

### Pasting HTML into the composer (tickets 04, 18)
1. Pasted HTML is data. The `htmd` conversion runs in Rust, and its output is Markdown text only. It is never rendered back as HTML in a non-sandboxed web view, and never passed to `NSAttributedString(html:)` (already rejected in research 04; that importer can fetch remote resources).
2. Remote image URLs or links in pasted content are not fetched automatically. Links stay text until the user clicks, and clicks only open `http(s)`. `file:`, `javascript:` and custom schemes are refused (match `LivePreview` rules, `remaining-security-fixes.md`).
3. Set size limits before parsing: pasteboard HTML byte cap, Markdown output cap, a nesting/depth cap in the converter, and a time budget. Over the limit, fall back to plain text.
4. Prompt-injection hygiene: pasted content is shown to the user as it will be sent, with no hidden text kept (strip `display:none`, comments, zero-width runs or make them visible), so the user can see what an agent will read.

### Previewing pasted files (ticket 04)
1. Check size *before* reading or mapping the file (research 04 found the 20 MiB check runs after mapping into memory). Use `QLThumbnailGenerator` / ImageIO thumbnails with a max pixel size. Never decode the full image just to make a thumbnail.
2. A preview never executes or opens the file. No `NSWorkspace.open` on an attachment. Symlinks and aliases are resolved and shown, and attachments outside the workspace show their real path.
3. HTML/SVG/Markdown previews reuse `ArtifactPreviewSecurity` (opaque-origin iframe, CSP, nonpersistent store). A text preview never interprets the file. Thumbnails are not written to persistent caches outside `~/.chorus`.
4. The image-before-file-URL ordering bug in `Composer.swift:336-355` (research 04) must not attach a different object than the one the user copied. Show the user what will be attached before it is sent.

### Review baseline (ticket 13) and the sub-session UI (ticket 11)
1. Commits made during a session by an **agent** must stay in review. A HEAD baseline would otherwise hide agent commits (Seatbelt allows agent writes to `.git/objects`/refs). The baseline is the HEAD at session start, or the last user "Mark reviewed", never a commit the agent made.
2. "Stop"/"message" controls show only for Chorus-started sub sessions owned by this session (reflecting P2-3). Provider sub-agents stay view-only.

## 5. Not checked

- Nothing was executed: no `cargo test`, no bench, no app run, no sandbox-exec reproduction. Every finding above is static evidence. Each P1/P2 still needs a regression test written in the fix ticket.
- No dependency advisory scan (`cargo-audit` still not installed). The `Cargo.lock` delta (107 lines, 2026-10-14) was not checked against RustSec.
- Not reviewed in depth: the ~1,990-line `ContentView.swift` and ~969-line `Engine.swift` diffs beyond the call sites above, `checkpoint_dag.rs`, `merkle.rs`, `mlx_runtime.rs`/`inference.rs`, `chorus-eval`, `DiffReviewerView`, `CheckpointTimelineView`. Spot checks found no new process, path or network sinks except those listed.
- Daemon HTTP routes beyond pairing were already covered by the audit (`fixes.md`) and were not re-reviewed.
