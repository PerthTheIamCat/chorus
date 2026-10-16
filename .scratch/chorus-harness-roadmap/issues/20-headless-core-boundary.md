# Headless core boundary

Type: grilling
Status: resolved
Blocked by: 05, 11

## Question

Should the Chorus core run as a headless daemon that the Swift app, the `chorus` CLI used by agents, and a future mobile client all connect to, instead of being an in-process FFI library? Decide the process model (in-app vs. launchd daemon vs. both), the IPC/API transport (Unix socket, local HTTP, gRPC; event push vs. `events_since`), auth for local and future remote clients, lifecycle when the app quits while agents run, and the migration path from today's C ABI.

## Answer

- **Process Model: Daemon-First Hybrid (`chorusd`)**:
  - The Rust core runs as a headless background daemon (`chorusd`). Swift UI, CLI, MCP server, and future mobile clients connect to this same engine boundary.
  - Auto-spawned by the Swift app on startup if not already running; can also run independently under `launchd` or via CLI for headless server environments.
  - Fault-tolerant and decoupled: If the macOS UI closes or crashes, active agents, subagents, and git worktrees continue processing uninterrupted. When the Swift app reopens, it restores complete state instantly.
- **IPC Transport & Push-Based Streaming**:
  - **Local Transport**: Unix Domain Socket at `~/.chorus/engine.sock` (fallback to localhost HTTP port `127.0.0.1:24678` for external tools).
  - **Replaces Polling C FFI**: Deprecates `chorus_events_since` polling in favor of high-throughput Server-Sent Events (SSE) streaming.
  - **Guaranteed Replay (`Last-Event-ID`)**: Clients reconnect with `Last-Event-ID: <ledger_row_id>`, causing `chorusd` to seamlessly replay missed events from the SQLite Ledger. Zero lost notifications.
- **Async Interaction State Machine**:
  - Replaces blocking C FFI callbacks with non-blocking ledger entries. Permission requests (tool execution, bash confirmation) are recorded as pending interactions in SQLite.
  - Any connected interface (Swift app, CLI command, or future mobile notification) can resolve the interaction asynchronously.
- **Application Quit & Lifecycle (`⌘Q`)**:
  - When closing the main window: The window dismisses while `chorusd` continues active work in the background.
  - When quitting the app with active agents: Prompts standard macOS dialog with two choices:
    1. *Keep Running in Background* (Default): GUI terminates; `chorusd` runs agents to completion and chimes / fires a native macOS UserNotification.
    2. *Stop All & Quit*: Sends graceful cancellation signals to all running bridges/subagents, cleans up ephemeral resources, and terminates `chorusd`.
- **Security & Authorization**:
  - Local clients restricted via Unix domain socket filesystem permissions (`0600`, owner-only) and an ephemeral token stored in the macOS Keychain (`~/.chorus/auth_token`).
  - Remote/mobile access gate uses mutual TLS (mTLS) or WireGuard/Tailscale encapsulation with QR-code ECDSA key-exchange pairing. No unauthenticated public network listeners.
