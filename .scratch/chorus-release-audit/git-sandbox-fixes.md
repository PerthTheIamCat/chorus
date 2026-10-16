# Git metadata sandbox corrections

Date: 2026-10-12. User-authorized targeted security fixes; no dependency, schema or public API change. Public-release acceptance remains open.

## Confirmed findings

| Priority | Reachable condition and source | Evidence and correction |
|---|---|---|
| P1 | `crates/chorus-acp/src/sandbox.rs`, previous Workspace carve-outs protected only cwd/.git/hooks and cwd/.git/config. A linked worktree gitfile points to an admin directory whose commondir identifies shared config/hooks; temp/cache metadata could inherit write allowances | Real sandbox regression failed before correction: shared config write succeeded. `git_guards` at line 255 now resolves nearest ancestor .git, relative gitdir and commondir, protecting real config, config.worktree, hooks and routing entries. Direct writes and atomic replacement are denied |
| P1 | Absolute protection rules could be bypassed by renaming an allowed parent directory, moving metadata to an unprotected pathname | New real sandbox regression reproduced ancestor relocation during implementation. `checked_policy` at line 93 adds literal directory-ancestor denies to seal these paths while retaining ordinary child writes |
| P2 | A quoted workspace pathname could cause the earlier deny rule to be silently omitted; metadata symlink targets were not independently protected | Path validation now rejects unrepresentable protection paths. Resolved symlink targets and original entries are protected, including .chorus; dangling protected links, invalid/oversized pointers and unavailable workspace paths fail closed |

Synthetic local fixtures establish reachable write-policy defects, not exploitation of a user installation. The hook/config impact is persistence through later Git invocations; no malicious hook was executed. The previous metadata write tests and before/after failures are recorded in this session's tool output.

## Behavior and compatibility

- Workspace mode still permits ordinary workspace files and index/ref metadata writes. It denies writes, deletion, replacement and relocation affecting protected config/hooks/routing/shadow paths. ReadOnly also denies writes to resolved external Git/admin/common metadata that might otherwise inherit temp allowances.
- Starting from a nested directory discovers the nearest repository and protects its .chorus tree. Normal directories, gitfiles and symlinked metadata are covered by fixtures.
- Metadata-pointer parsing reads only gitfile/commondir routing data, up to 4,097 bytes to enforce a 4,096-byte limit. It does not read config contents, credential files or private databases, and does not invoke Git or hooks to discover paths.
- Public `policy` retains its String signature and returns a deny-all profile on discovery failure. `wrap` uses the checked builder and returns the error before spawning; restrictive CLI policies already fail closed on that error.
- Invalid/dangling metadata, non-UTF-8 or unrepresentable path characters now prevent a restrictive turn from starting. Git initialization, metadata relocation, configuration edits and hook installation must occur outside this restricted agent execution. No host Git operations or user repository data were modified by the synthetic tests.
- Fixed-path policy generation does not establish protection against concurrent external changes by other same-user processes. Custom core.hooksPath, included configuration, alternate environment-selected repositories, nested repositories and other executable Git configuration require separate coverage. Reads, network and existing agent cache/config allowances retain their previous policy; this is not a comprehensive capability sandbox.

## Validation

- `cargo test -p chorus-acp --locked`: 14 passed, including real macOS sandbox-exec checks for linked/shared metadata, nested cwd, symlink aliases, routing replacement, ancestor relocation, ReadOnly external index denial and fail-closed invalid inputs. Workspace/index positive controls passed.
- `cargo test -p chorus-core -p chorus-cli -p chorus-acp -p chorus-daemon --locked`: 221 passed, 3 ignored. Filtered counts include a subprocess regression invoking itself with --exact.
- `cargo build -p chorus-ffi --locked`: passed.
- `cargo clippy -p chorus-acp --all-targets --locked --no-deps`: 0 errors, one pre-existing warning in antigravity_bridge.rs:133. No unrelated cleanup.
- `rustfmt --edition 2021 --check crates/chorus-acp/src/sandbox.rs` and scoped `git diff --check`: passed.
- No application installation, dependency advisory scan, performance benchmark or new Swift build was run in this Rust-only correction round. Existing unrelated changes were preserved. Tests use a synthetic documented Git layout rather than creating/committing a user repository.

## Official source

[Git repository layout](https://git-scm.com/docs/gitrepository-layout) confirms that .git can be a gitfile pointing at a separate directory and that commondir redirects shared metadata. The implementation resolves relative pointers against the containing metadata file's directory and verifies actual Seatbelt enforcement, rather than treating the policy text as sufficient evidence.

## Follow-up

[Which remaining capability, integrity and release checks need proof?](issues/10-remaining-release-coverage.md) retains unresolved security coverage. [Performance measurement evidence](issues/04-performance-evidence.md) and [release policy](issues/06-release-policy.md) remain open.
