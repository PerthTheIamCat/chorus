# Which distribution channel and trust policies should the release support?

Parent: ../map.md
Labels: wayfinder:grilling
Type: grilling
Mode: HITL
Status: resolved
Assignee: perth
Blocked by: none

## Question

With the user, settle direct Developer ID distribution versus Mac App Store, supported macOS versions, remote peer privileges, and data retention/export expectations. These choices determine which platform requirements apply and which behaviors are intended. Use the research checklists as context; investigators may collect conditional evidence before these choices, but final release gates must not assume the answers. Do not change signing, sandboxing or runtime configuration.

## Answer

Confirmed with user on 2026-10-14:
- **Distribution Channel**: Direct Developer ID signed `.dmg` with Hardened Runtime and Apple Notarization (`notarytool`). Mac App Store is excluded due to sandboxing restrictions on agent CLI/subprocess execution.
- **Platform Support**: macOS 14.0+ (Sonoma / Sequoia) targeting Apple Silicon (`arm64`).
- **Remote Peer Privileges**: Remote companion pairing initiated strictly via host-controlled 0600 Unix domain socket using cryptographic OS entropy; mobile devices receive paired bearer tokens for async approvals and status observation, without host admin privileges.
- **Data Retention & Privacy**: Strictly local-first SQLite Ledger (`~/.chorus/ledger.db`). Zero external telemetry or cloud sync.
