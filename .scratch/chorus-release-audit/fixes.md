# Confirmed daemon and packaging corrections

Date: 2026-10-12. User authorized implementation after the initial planning session. These results cover the daemon and packaging paths below; they are not a complete application audit or a measured desktop performance baseline.

## Findings and resulting behavior

| Priority | Confirmed condition | Correction | Evidence |
|---|---|---|---|
| P1 | TCP routes exposed ledger events, approvals, lifecycle controls and pairing administration without authentication; wildcard CORS enabled browser access | Require paired bearer tokens on private TCP routes; reject browser Origin; reserve pairing administration, lifecycle changes and shutdown for the owner-only Unix socket | `tests/security.rs`: unauthenticated route matrix, paired-client administration denial, Origin cases; existing Unix socket integration tests |
| P1 | Device listing returned persistent bearer tokens | Return device metadata only; token is returned only by a successful pairing completion | Device-list regression checks absence of auth_token and credential text |
| P1 | Pairing and auth tokens used a non-cryptographic generator | Use 32 bytes of OS entropy through existing libc/getentropy; return an error on entropy failure. No dependency or lockfile changes | OS entropy call inspected; handshake, replacement, one-time-use, expiry and revoke tests. An exploit predicting the former tokens was not attempted |
| P1 | Plain TCP could bind to all interfaces and expose traffic/credentials directly | Reject non-loopback addresses before creating database/socket; encrypted tunnel deployment must be designed separately | 0.0.0.0 rejection regression; loopback fallback integration |
| P2 | An authenticated SSE stream remained usable after device revocation | Recheck authorization before replay/live writes and keepalives; close the stream on revocation | Existing-stream revoke regression. Idle revocation is checked on the next keepalive, at most about 10 seconds; an in-flight socket write can additionally wait for its write timeout |
| P2 | Request lines/headers, worker threads and per-client event queues had no caps | Cap request headers at 16 KiB, active connections at 64, event queues at 256; reject duplicate/invalid Content-Length and unsupported Transfer-Encoding; disconnect slow event consumers without blocking healthy ones | Header negative tests, connection-permit cap/reuse test, slow/healthy subscriber test. This bounds resource use; it does not guarantee availability against sustained connection flooding |
| P2 | SSE replay held the shared SQLite mutex while writing to a client; live events ignored the requested session filter | Release DB lock before network I/O; filter live events by session | Source inspection for lock scope; session-filter integration regression |
| P2 | Pairing initiation advertised a fixed port rather than the actual configured listener | Advertise the bound endpoint; fail pairing initiation when HTTP is disabled | Ephemeral-port regression in pairing HTTP test |
| P2 | Build swallowed signing failure; packager could use an old installed bundle and silently re-sign failed verification | Sign embedded executables before the bundle and fail on signing/verification errors; package the current SwiftPM release output and stop when verification fails | Bash syntax check and isolated packager fixture with stub tools, fresh-build marker, failed verification and successful packaging paths |

## Compatibility and operational effect

- Native owner control continues over the 0600 Unix socket. Unauthenticated HTTP status/control clients now receive 401; paired HTTP clients cannot manage host lifecycle or pairing administration. Existing DaemonClient.request supports supplying an Authorization header.
- Direct non-loopback HTTP is refused. Remote access needs an encrypted transport terminating on loopback, a valid pairing token and a correctly configured externally reachable pairing endpoint. No tunnel or TLS deployment was configured here.
- PairingManager.initiate_session now returns Result so entropy failure is explicit; all repository callers were updated. Pairing credentials remain in memory, as before.
- Slow SSE clients are disconnected and must reconnect. Durable ledger events can be replayed via Last-Event-ID; custom broadcast-only events have no durable replay guarantee.
- The current desktop RemotePairingView generates its own QR token and removes devices locally; wiring that view to the actual daemon remains an audit/integration item.
- Development signing remains ad-hoc. Developer ID, Hardened Runtime, notarization and Gatekeeper acceptance remain release work; these changes do not certify the app for public distribution.

## Validation

- `cargo test -p chorus-daemon --locked`: 19 tests passed.
- `rustfmt --edition 2021 --check` on the six changed Rust files: passed.
- `cargo clippy -p chorus-daemon --all-targets --locked --no-deps -- -D warnings`: blocked by four existing should_implement_trait findings in interaction.rs/lifecycle.rs, which were not changed.
- The same Clippy command with `-A clippy::should_implement_trait`: passed, with that pre-existing lint explicitly excluded for validation only.
- `bash -n apps/macos/build.sh apps/macos/package-dmg.sh`: passed.
- Isolated Python/temp-directory packaging fixture: passed fresh-build selection and fail-before-DMG on signature rejection. All fixture artifacts were cleaned up.
- No full app build, real signing/notarization, installation, model downloads or performance benchmarks were run. The installation scripts have existing side effects and were not executed.

## Primary sources used

- [fastrand documentation](https://docs.rs/fastrand/latest/fastrand/): its generator is not cryptographically secure.
- [getrandom's getentropy backend](https://docs.rs/crate/getrandom/latest/source/src/backends/getentropy.rs): OS entropy through libc/getentropy, supported on macOS since 10.12; requests stay below its size limit.
- [MCP transport security](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports): authentication, Origin protection and loopback binding are relevant local-server controls. Chorus daemon is not itself asserted to implement this MCP transport.
- [Apple code signing guidance](https://developer.apple.com/library/archive/technotes/tn2206/_index.html): sign nested code before the containing bundle; use deep verification rather than deep signing.

## Remaining investigation

Agent permission enforcement and sandbox reachability, updater/model integrity, preview isolation, sensitive data storage/logs, stale socket cleanup and filesystem confinement, dependency advisories, and desktop/local-model profiling remain unverified. SSE replay still materializes its result set; high-volume memory behavior needs measurements and pagination design. No security/performance acceptance gate for the whole app is marked complete.
