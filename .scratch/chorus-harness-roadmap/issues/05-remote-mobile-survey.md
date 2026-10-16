# Remote and mobile control survey

Type: research
Status: resolved
Blocked by:

## Question

How do existing tools let a phone drive a desktop coding agent? Cover Claude Code remote/mobile, Happy (Claude Code mobile client), Omnara, Codex mobile / cloud tasks, Cursor background agents on web and mobile, and any other well-regarded open-source equivalents. Record the architecture (local daemon, relay server, tunnel, E2E encryption), device pairing and auth, the event stream protocol, which actions are allowed remotely (approve permissions, send messages, view diffs), and notification delivery. The goal is to find what the Chorus core must expose (a headless daemon and its API surface) so a mobile client can be added later without redesign.

## Answer

- **Architectural patterns**: Surveyed Claude Code (`/remote-control`), Happy (`slopus/happy`), Omnara (`omnarad`), Codex desktop/mobile, Cursor background agents, OpenCode Mobile, and CCGram. Implementations cluster into: (1) Local host with outbound-only relay (Claude Code, Happy, Codex), (2) Direct peer-to-peer / mesh tunnel (OpenCode, Tailscale), and (3) Cloud VM / decoupled control plane (Cursor, Omnara).
- **Security & Pairing**: Proprietary tools terminate TLS at vendor cloud relays with account OAuth/QR pairing. Privacy-centric tools (Happy) use offline QR-code pairing (Curve25519 key exchange) and zero-knowledge end-to-end encryption (TweetNaCl / AES-GCM), preventing the relay from inspecting code or prompts.
- **Event Streaming & Disconnect Resilience**: Real-time communication uses WebSockets or Server-Sent Events (SSE). Mobile network drops require sequence-numbered event replay (`since={seq}`), which maps 1:1 onto Chorus's sequential SQLite ledger without extra synchronization infrastructure.
- **Decoupled Permission Loop**: Synchronous blocking function callbacks (like Chorus's current C FFI `PermissionCallback`) must be replaced with an asynchronous `Interaction` state machine (`pending` / `resolved`) so turns do not block native OS threads while mobile devices are locked or offline.
- **Remote Action Surface**: Mobile is consistently a supervision and steering interface (send messages, voice prompts, approve/reject tool calls, inspect syntax-highlighted diffs, trigger handoffs/aborts); direct file editing is intentionally left to desktop IDEs.
- **Notification Delivery**: Because mobile OSes suspend background WebSockets after ~30s, out-of-band push notifications (APNs/FCM) triggered via webhook hooks upon `interaction.created` or `turn.completed` are required.
- **Headless Chorus Core Boundary**: Chorus core should expose a local headless daemon (`chorusd`) over Unix Domain Socket and HTTP/SSE/WebSocket with REST endpoints for sessions, turn execution, asynchronous interactions/approvals, working diffs/rejections, and webhook notification triggers. This allows immediate mobile usage via Tailscale now and a zero-knowledge relay wrapper in Phase 6 without redesigning the core.

See complete findings and API specifications in [../research/05-remote-mobile-survey.md](../research/05-remote-mobile-survey.md).
