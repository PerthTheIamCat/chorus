# Remote and Mobile Control Survey for AI Coding Agents

This survey investigates how modern developer tools enable mobile phones and remote web clients to drive desktop and cloud coding agents. It records the underlying architecture (local daemon, relay server, tunnels, E2E encryption), pairing and authentication mechanisms, event streaming protocols, remote action surfaces (permissions, messages, diffs), and notification delivery. Finally, it derives the exact headless daemon architecture and API surface that the Chorus core must expose so a mobile companion can be introduced in Phase 6 without refactoring the core.

---

## 1. Executive Summary & Industry Landscape

Between 2024 and 2026, the coding agent interaction pattern shifted from synchronous desktop terminal sessions to **asynchronous, remote-supervised agentic workflows**. Developers assign multi-hour coding tasks and step away from their workstations, expecting their phones to act as an "approval pager" and remote steering wheel.

Existing implementations cluster into three primary architectures:

```
[Pattern A: Local Outbound Relay] (Claude Code, Happy, Codex Desktop)
+----------------+      Outbound TLS / WS       +-------------------+      Mobile / Web
| Local Desktop  | ---------------------------> | Cloud Relay       | <--------------------+
| Daemon & Agent |                              | (Zero or Encrypted|                      |
+----------------+                              |  State)           |                      |
                                                +-------------------+                      |
                                                                                           |
[Pattern B: Direct Peer-to-Peer Tunnel] (OpenCode Mobile, Tailscale)                       |
+----------------+      WireGuard / Tunnel                                                 |
| Local Desktop  | <=======================================================================+
| HTTP + SSE     |
+----------------+

[Pattern C: Centralized Control Plane / Cloud Sandboxes] (Omnara, Cursor Cloud Agents)
+----------------+      Agent loop in Cloud VM  +-------------------+
| Ephemeral VM / | <--------------------------- | Control Plane     | <--------------------+
| Sandbox Worker |                              | (Postgres + API)  |
+----------------+                              +-------------------+
```

1. **Local Host with Outbound Relay (Pattern A)**: The local machine remains the execution environment and source of truth (files, local tools, MCP servers, git state). The local daemon connects *outbound* via TLS/WebSocket to a cloud relay. The mobile client connects to the relay. In privacy-focused tools (Happy), payloads are end-to-end encrypted with zero-knowledge keys.
2. **Direct Peer-to-Peer Tunneling (Pattern B)**: The local daemon exposes an HTTP/SSE API directly on localhost/LAN, and users traverse NAT using mesh tunnels (Tailscale, Cloudflare Tunnels, WireGuard). No intermediary relay server is operated by the vendor.
3. **Decoupled Control Plane & Cloud Sandboxes (Pattern C)**: Agent execution runs in ephemeral cloud containers/VMs (Cursor) or workers decoupled from machines (Omnara). A centralized database stores durable state. Mobile devices communicate with the cloud control plane directly; local machines participate only as connected execution targets via a machine daemon (`omnarad`).

For **Chorus**, whose foundational architectural principles demand that **the ledger is the single source of truth** (Rule 2.2) and **every logic piece lives in Rust** (Rule 2.3), the target architecture is a **local headless daemon exposing a structured HTTP/WebSocket API**. This core daemon can serve local macOS Swift UI over Unix Domain Sockets or localhost, while seamlessly plugging into an outbound relay (or tunnel) for mobile clients.

---

## 2. Comprehensive Tool Survey

### 2.1 Claude Code Remote Control (`/remote-control`)

* **Primary Sources**:
  - [Claude Code Documentation: Overview and Platforms](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/overview)
  - [Claude Code Remote Control Guide](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/remote-control)
  - [Anthropic Engineering: Managing Long-Running Agent Sessions](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/sessions)
  - [Claude Code Diagnostics & Configuration](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/settings)

* **Architecture**:
  - **Local Host Execution**: The agent process, bash execution engine, filesystem access, git state, and local Model Context Protocol (MCP) servers run entirely on the user's local workstation.
  - **Outbound-Only Relay**: The local CLI process opens an outbound TLS connection (`wss://` WebSocket gateway) to Anthropic's relay infrastructure. No inbound firewall ports or port-forwarding rules are required.
  - **Relay Role**: The Anthropic cloud relay acts as a rendezvous broker routing messages between the remote web client (`claude.ai/code`) or mobile app and the local terminal session. Session transcripts are buffered on Anthropic servers during the session and expire approximately four hours after the local daemon disconnects.

* **Device Pairing & Authentication**:
  - **Account Level**: Requires an authenticated Claude Pro, Max, Team, or Enterprise subscription (API-key-only accounts are not supported for Remote Control).
  - **Session Pairing**: Initiated by running `/remote-control` (or `/rc`, or `claude remote-control`). The CLI prints a QR code and a pairing URL: `https://claude.ai/code/<session_id>`.
  - **Session Identity**: Environment variables expose the session state: `$CLAUDE_CODE_BRIDGE_SESSION_ID` stores the active bridge session ID, while `$CLAUDE_CODE_REMOTE="true"` marks remote web executions. Session names can be prefixed via `--remote-control-session-name-prefix`.

* **Event Stream Protocol**:
  - Bidirectional WebSocket streaming between the CLI and Anthropic's gateway.
  - Streaming events transmit model thought chunks, tool call requests, execution logs, and turn completion boundaries.

* **Allowed Remote Actions**:
  - **Send Prompts**: Inject new user instructions or steering messages into the running turn loop.
  - **Permission Approvals**: Approve or reject sensitive tool calls (shell execution, file modifications).
  - **Session Control**: Interrupt/abort active generation, view real-time terminal output and progress.
  - **Diffs & Files**: View generated diffs inline in the transcript; full IDE file editing is not supported on mobile.

* **Notification Delivery**:
  - Push notifications sent through the official Claude mobile app (iOS and Android) when an agent requires approval or completes a multi-step objective.

---

### 2.2 Happy (`slopus/happy`) — Mobile Companion for Claude Code & Codex

* **Primary Sources**:
  - [Happy GitHub Repository (`slopus/happy`)](https://github.com/slopus/happy)
  - [Happy Engineering Official Website](https://happy.engineering)
  - [Happy iOS App Store Listing](https://apps.apple.com/us/app/happy-claude-code-client/id6748571505)
  - [Happy Google Play Store Listing](https://play.google.com/store/apps/details?id=com.ex3ndr.happy)

* **Architecture**:
  - **Host Wrapper / Daemon**: Happy provides an open-source CLI (`npm install -g happy-coder`) and desktop application (`slopus/happy-desktop`) that wraps existing coding agents (`happy` for Claude Code, `happy codex` for Codex, `happy gemini` for Gemini CLI). A background daemon (`happy daemon`) maintains the connection.
  - **Relay Server**: Uses an open-source relay (`happy-server`). A free hosted relay is provided at `happy-api.slopus.com`, but the entire relay can be self-hosted in Docker on private infrastructure.
  - **Zero-Knowledge End-to-End Encryption (E2EE)**: Unlike official proprietary relays, Happy implements zero-trust end-to-end encryption. Data is encrypted locally before hitting the relay. The relay stores and transmits only opaque encrypted binary blobs.
  - **Cryptographic Primitives**: Uses TweetNaCl (Curve25519 key exchange, XSalsa20-Poly1305 symmetric encryption) and AES-256-GCM with per-session ephemeral keys. The relay operator cannot inspect source code, prompts, or tool outputs.

* **Device Pairing & Authentication**:
  - **Keypair Generation**: The mobile app generates an asymmetric keypair locally on first launch without requiring email, password, or third-party OAuth.
  - **QR Code Handshake**: The desktop CLI/app renders a QR code encoding the session endpoint, public identity, and shared pairing secret. Scanning the QR code establishes the shared secret between the phone and desktop.

* **Event Stream Protocol**:
  - WebSocket synchronization stream over TLS to the relay. Messages, terminal diffs, and permission states are serialized, encrypted as NaCl envelopes, and pushed bidirectionally.

* **Allowed Remote Actions**:
  - **Interactive Approvals**: Real-time modal prompts for file modifications and command execution approvals.
  - **Message Submission**: Voice-to-text dictation and typed prompt submission.
  - **Visual Diff Inspection**: Syntax-highlighted, colored diff viewer rendered natively on iOS/Android.
  - **Harness Switching**: Multi-provider switching across Claude, Codex, and Grok within a unified session.

* **Notification Delivery**:
  - Direct APNs (Apple Push Notification service) and FCM (Firebase Cloud Messaging) notifications triggered by the local host via `happy notify` through the relay when agent halts for user input.

---

### 2.3 Omnara (`omnara-ai/omnara`) — Open-Source Managed Agent Platform

* **Primary Sources**:
  - [Omnara GitHub Repository (`omnara-ai/omnara`)](https://github.com/omnara-ai/omnara)
  - [Omnara Official Documentation](https://docs.omnara.com)
  - [Omnara REST & Streaming API Reference](https://docs.omnara.com/api/overview)
  - [Omnara Machine Daemon (`omnarad`) Guide](https://docs.omnara.com/machines/daemon)

* **Architecture**:
  - **Decoupled Control Plane**: Omnara separates agent orchestration from execution infrastructure. The control plane runs a FastAPI/Node backend with PostgreSQL as the single source of truth for the agent's durable event log.
  - **Machine Daemon (`omnarad`)**: Developers install a lightweight daemon on their local machine, VM, or container (`curl -fsSL https://app.omnara.com/install/omnarad.sh | sh`). The daemon establishes an outbound connection to the Omnara control plane, exposing the host's filesystem and shell without requiring inbound ports.
  - **Stateless Workers**: Model context assembly and LLM calls are executed by workers that dispatch tool actions to the registered `omnarad` instance.

* **Device Pairing & Authentication**:
  - **Authentication**: Organization/Project tokens (`OMNARA_TOKEN`) and user session cookies. CLI authentication via `npx omnara login` opens a browser verification flow.
  - **Machine Registration**: `omnarad` pairs with an organization using a machine enrollment token issued in the web dashboard.

* **Event Stream Protocol**:
  - **Server-Sent Events (SSE)**: Clients stream live agent events via `GET /api/v1/events/stream` (consumed via `@omnara/sdk` `openAgentEventStream`).
  - **HTTP REST Endpoints**: Standard JSON API for commands and state queries:
    - `POST /api/v1/agents` (launch agent)
    - `POST /api/v1/agents/{id}/input` (send message)
    - `GET /api/v1/interactions` (list pending approvals/questions)
    - `POST /api/v1/interactions/{id}/resolve` (submit human decision)

* **Allowed Remote Actions**:
  - **First-Class Interaction Approvals**: Omnara models human-in-the-loop decisions as explicit `Interaction` resources. When an agent requests a sensitive command, an interaction is created. The mobile app or web console resolves it via `POST /interactions/{id}/resolve` with options: approve, reject, or text guidance.
  - **Steering**: User input can be sent with `cancel_open_interactions: true` to interrupt an agent mid-thought.
  - **File Artifacts & Diffs**: Download and inspect artifacts produced during turns.

* **Notification Delivery**:
  - Push notifications on mobile, webhooks to external services, and native bots in Slack and Discord that render approval buttons directly in team channels.

---

### 2.4 OpenAI Codex Mobile / "Control this Mac or PC"

* **Primary Sources**:
  - [OpenAI Help: Remote Connections in ChatGPT Desktop](https://help.openai.com/en/articles/10009584-remote-connections-control-this-mac-or-pc)
  - [OpenAI Platform: Codex Tasks and Agents](https://platform.openai.com/docs/guides/agents)
  - [ChatGPT Mobile App Reference](https://openai.com/chatgpt/)

* **Architecture**:
  - **Host Integration**: The ChatGPT macOS and Windows desktop applications include an embedded agent engine capable of running bash commands, editing code, and executing tests on the local machine.
  - **Cloud Coordination**: The desktop app maintains an authenticated outbound connection to OpenAI cloud coordination servers. The ChatGPT mobile app on iOS/Android communicates with the same coordination cloud.
  - **Keep-Awake Management**: Because the local host must remain operational, the desktop app introduces a dedicated system assertion toggle: "Keep this Mac awake while remote connections are active".

* **Device Pairing & Authentication**:
  - **Account Boundary**: Both devices must be logged into the same ChatGPT workspace (Plus, Pro, Team, or Enterprise).
  - **QR Code Cryptographic Handshake**: In **Settings > Connections > Control this Mac or PC**, the user clicks "Set up", which displays a dynamic QR code. Scanning the QR code with the mobile app performs device authorization and establishes passkey/SSO session linking.

* **Event Stream Protocol**:
  - Proprietary streaming infrastructure over HTTPS/WebSockets synchronizing prompt inputs, tool calls, bash output buffers, and diff summaries.

* **Allowed Remote Actions**:
  - **Task Delegation**: Trigger new tasks targeting the local computer or cloud sandbox.
  - **Supervision & Approvals**: Approve or deny requested bash scripts, package installations, and git commands.
  - **Diff & Screenshot Inspection**: View colored diffs of edited files, live terminal logs, and captured GUI screenshots.

* **Notification Delivery**:
  - Native iOS/Android push notifications via APNs and FCM alerting the user when an action gate requires confirmation or when a multi-step task completes.

---

### 2.5 Cursor Background Agents (Web & Mobile)

* **Primary Sources**:
  - [Cursor Documentation: Background & Cloud Agents](https://docs.cursor.com/agent/overview)
  - [Cursor Agent Dashboard](https://cursor.com/agents)
  - [Cursor iOS App Store Listing](https://apps.apple.com/app/cursor-ai/id6740000000)

* **Architecture**:
  - **Bimodal Execution Model**:
    1. **Cloud Agents**: Cursor provisions ephemeral cloud Linux VMs pre-loaded with the user's GitHub repository. The agent runs autonomously in the cloud even if the user's laptop is powered down.
    2. **Local Remote Control**: The local Cursor desktop editor can act as a host, paired with the mobile iOS app or web browser to supervise active editor-based agent runs.
  - **Web Dashboard / Mobile Client**: Accessible via `cursor.com/agents` (as a responsive web app or PWA) and a native iOS app.

* **Device Pairing & Authentication**:
  - Single Sign-On (SSO) via GitHub, Google, or Cursor account. Cloud agents pair via GitHub App permissions for repository read/write access. Local remote control pairs via desktop configuration.

* **Event Stream Protocol**:
  - High-performance WebSocket stream delivering execution timelines, agent chain-of-thought, terminal outputs, and file modifications.

* **Allowed Remote Actions**:
  - **Directing Agents**: Launching new features/bugfix tasks, supplying clarifying prompts, or canceling runs.
  - **Voice Prompts & Annotations**: Voice input dictation and image attachment with visual canvas annotations.
  - **Reviewing PRs & Diffs**: Inspecting complete unified/split diffs and approving pull requests created by the agent.
  - **Constraint**: Mobile explicitly disallows direct in-file text editing; it is strictly a supervision and review interface.

* **Notification Delivery**:
  - Mobile push notifications (iOS app), GitHub PR notifications, and Slack/Discord webhook alerts upon task completion.

---

### 2.6 Other Well-Regarded Open-Source Equivalents

#### A. OpenCode & OpenCode Mobile (`dzianisv/opencode-mobile`)
* **Primary Sources**:
  - [OpenCode Official Website](https://opencode.ai)
  - [OpenCode Mobile GitHub](https://github.com/dzianisv/opencode-mobile)
* **Architecture**: The core engine (`opencode serve`) runs a persistent Bun/Hono server on the host machine exposing an OpenAPI 3.1 specification. The mobile client (built in React Native/Expo) connects over LAN or secure tunnel (Tailscale, Cloudflare Tunnels, ngrok).
* **Protocol**: Real-time **Server-Sent Events (SSE)** for agent thought process, tool invocations, and diffs; REST HTTP requests for prompts and approvals.
* **Key Characteristic**: Zero vendor lock-in; no third-party relay required if a mesh VPN like Tailscale is present.

#### B. CCGram (`alexei-led/ccgram`)
* **Primary Sources**:
  - [CCGram GitHub Repository](https://github.com/alexei-led/ccgram)
* **Architecture**: A minimalist terminal-to-chat bridge. It connects the **Telegram Bot API** directly to local `tmux` sessions running Claude Code, Codex, or other CLI tools on a desktop or home server.
* **Protocol & Pairing**: Long-polling over HTTPS via Telegram Bot API with token authentication. Each Telegram forum topic maps to a separate `tmux` window.
* **Allowed Actions**: Remote prompting, viewing streamed terminal output chunks, and approving prompts using Telegram inline keyboard buttons (`[Approve]`, `[Reject]`).
* **Notification**: Native Telegram push notifications with zero extra push infrastructure.

#### C. Paseo (`getpaseo/paseo`)
* **Primary Sources**:
  - [Paseo GitHub Repository](https://github.com/getpaseo/paseo)
* **Architecture**: Multi-agent cross-platform orchestrator running a local daemon that interfaces with Claude Code, OpenCode, and Pi. Connects to mobile via an encrypted rendezvous relay.

---

## 3. Comparative Architecture Matrix

| Dimension | Claude Code Remote Control | Happy (`slopus/happy`) | Omnara (`omnara-ai/omnara`) | OpenAI Codex Desktop/Mobile | Cursor Background Agents | OpenCode Mobile | CCGram |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Execution Host** | Local Workstation | Local Workstation | Local Machine (`omnarad`) or Cloud | Local Workstation or Cloud Sandbox | Ephemeral Cloud VM (or Local) | Local Workstation or Server | Local Workstation (`tmux`) |
| **Network Topology** | Outbound TLS to Vendor Relay | Outbound TLS to Relay (Self-hostable) | Outbound to Control Plane | Outbound TLS to OpenAI Cloud | Cloud-native or Local Outbound | Direct HTTP/LAN or Mesh Tunnel (Tailscale) | Outbound HTTPS Long-poll to Telegram |
| **End-to-End Encryption** | No (TLS to Anthropic; server holds transcript) | **Yes (TweetNaCl / AES-GCM Zero-Knowledge)** | No (TLS to Control Plane; Postgres stores state) | No (TLS to OpenAI) | No (TLS to Cursor) | Yes via Tailscale WireGuard (or plain TLS) | End-to-end between host and Telegram servers |
| **Device Pairing** | Account login + URL/QR code | Offline QR Code (Public Key + Secret) | Project API Token + CLI OAuth | Workspace Account + Settings QR Code | GitHub / Cursor Account SSO | Direct IP / Tailscale IP / Auth Token | Telegram Bot Token + Chat ID allowlist |
| **Event Stream Wire Protocol** | WebSocket (`wss://`) | WebSocket (`wss://`) | Server-Sent Events (`GET /events/stream`) | WebSocket / HTTPS | WebSocket | Server-Sent Events (SSE) | Telegram Bot Polling / Webhook |
| **Human-in-the-Loop Interaction** | CLI pauses; prompts for input on Web/Mobile | Interactive prompt envelope via WebSocket | First-class `Interaction` REST resource | Modal approval request on Mobile app | Web/App review status & PR approval | Modal permission request via REST | Telegram Inline Keyboard buttons |
| **Remote Actions Allowed** | Prompt, approve/reject, abort, view diffs | Prompt, voice, approve/reject, colored diffs | Prompt, resolve interactions, stream events, artifacts | Prompt, approve bash/tools, diffs, view screenshots | Prompt, voice, review diffs, approve PRs (no file edit) | Prompt, approve/reject tools, view diffs | Prompt, inline button approve/reject, bash logs |
| **Notification Delivery** | Official Claude App Push (APNs/FCM) | Relay APNs/FCM push (`happy notify`) | Mobile Push, Webhooks, Slack/Discord bots | Official ChatGPT App Push (APNs/FCM) | Mobile App Push (APNs), Slack, GitHub alerts | OS Push via Expo/APNs or Local LAN | Native Telegram Push |
| **Offline Resilience** | Disconnects if host sleeps | Disconnects if host sleeps | Durable: Postgres saves state; resumes when reconnected | Disconnects if host sleeps (unless Keep-Awake set) | Unaffected by host sleep (runs in Cloud VM) | Disconnects if host sleeps | Host must stay awake |

---

## 4. Key Architectural Patterns & Lessons for Chorus

### 4.1 Network Topology: Relay vs. Mesh Tunnel vs. Direct

A mobile phone is almost never on the same local subnet as a developer's desktop machine when the developer is away from their desk. It operates behind cellular carrier NATs (CGNAT), changing IP addresses across cell towers.

Three options exist for NAT traversal:
1. **Direct Mesh Tunnel (e.g., Tailscale / WireGuard)**:
   - *How it works*: User installs Tailscale on Mac and iPhone. The phone connects directly to `http://macbook.tailnet-xyz.ts.net:8080`.
   - *Pros*: Zero external server dependencies; end-to-end encrypted by WireGuard; Chorus does not need to run or maintain any relay cloud infrastructure.
   - *Cons*: High friction for non-technical users (requires Tailscale installation and account).
2. **Vendor-Operated Outbound Relay (Claude Code / Codex model)**:
   - *How it works*: Desktop connects outbound to `relay.chorus.sh`. Phone connects to `relay.chorus.sh`.
   - *Pros*: Zero configuration. Works anywhere instantly via QR code.
   - *Cons*: Operational cost; privacy concerns if relay can see plaintext code.
3. **Zero-Knowledge Encrypted Relay (Happy model)**:
   - *How it works*: Desktop generates ephemeral Curve25519 keypair. QR code transfers public key and secret to phone. Desktop and phone encrypt all payloads with NaCl `box` / `secretbox`. Relay only passes encrypted blobs.
   - *Pros*: Zero configuration, zero privacy risk, self-hostable relay.

**Conclusion for Chorus**: In Phase 1–5, Chorus core should not build or operate a cloud relay. However, Chorus core **must expose a clean, local HTTP/SSE/WebSocket server** on localhost/Unix socket. A developer running Tailscale can immediately use it on mobile on day one without any extra infrastructure. When Phase 6 builds the official mobile app, an outbound zero-knowledge relay (Pattern 3) can simply wrap this local API.

### 4.2 Decoupling the Permission/Approval Loop

In Chorus today, tool permissions are handled synchronously via an FFI function pointer callback:
```rust
// Current chorus-ffi design (crates/chorus-ffi/src/lib.rs):
pub unsafe extern "C" fn chorus_run_turn_interactive(
    e: *mut ChorusEngine,
    agent: *const c_char,
    prompt: *const c_char,
    callback: PermissionCallback, // Blocking C function pointer!
    ctx: *mut std::ffi::c_void,
) -> *mut c_char;
```
When `callback` is called, the Rust thread blocks until the Swift UI returns `0` or `1`.

This synchronous in-process callback model **fails completely for remote and mobile control**:
1. If the user is on their phone, the mobile device might be locked or asleep when the tool executes.
2. The turn thread cannot block a native OS thread indefinitely waiting for a mobile network packet that might take minutes to arrive.
3. If the Swift app terminates, the entire engine and session die.

**Omnara's lesson**: Model approvals as **first-class, asynchronous `Interaction` resources**:
- When an agent bridge requests permission (e.g. bash execution, file modification, memory mutation), Chorus core records an `InteractionPending` event in the ledger.
- The turn loop suspends asynchronously via an `async` channel / future.
- The core broadcasts an event `interaction.created` with an ID, tool name, parameters, and affected diff.
- A client (Swift desktop, mobile app, or webhook) calls `POST /interactions/{id}/resolve` with `{ decision: "allow" | "deny" | "reply", feedback: "..." }`.
- The core resolves the future, logs `InteractionResolved` to the ledger, and resumes the agent.

### 4.3 Ledger-Driven Event Replay & Disconnect Resilience

Mobile network connections drop constantly (elevators, subway, app backgrounding). iOS suspends background apps after 30 seconds, terminating open WebSockets.

Because **the ledger is Chorus's single source of truth** (Rule 2.2), Chorus has an inherent competitive advantage over stateless terminal multiplexers:
- Every event in Chorus has a monotonically increasing sequence ID (`seq`).
- A client reconnecting after 10 minutes does not need complex state reconciliation. It simply calls:
  `GET /events?since=42` (or opens an SSE stream `GET /events/stream?since=42`).
- Chorus streams all missed events since sequence 42 and continues live streaming.

### 4.4 Notification Triggers

Because mobile operating systems terminate background sockets, the mobile client **cannot rely on an open socket to receive notifications**. An out-of-band notification trigger is required.
Chorus core does not need to communicate directly with Apple APNs or Google FCM. Instead, Chorus core must expose a **Webhook / Event Hook emitter**:
- When an interaction is created (`interaction.created`) or a turn completes (`turn.completed`), Chorus core dispatches a JSON payload to configured local hooks or outbound webhook URLs.
- A local utility or relay server picks up this webhook and routes it to APNs, Pushover, Telegram, or Discord.

---

## 5. What Chorus Core Must Expose: Headless Daemon & API Surface

To ensure a mobile client can be introduced in Phase 6 without redesigning `chorus-core` or `chorus-cli`, the core boundary (Ticket 20) must adhere to the following headless daemon specifications.

### 5.1 Process Model & Transports

Chorus core should compile to a standalone headless binary (`chorusd`) or an embedded background server runtime within `chorus-cli`:
- **Unix Domain Socket (UDS)**: On macOS, `chorusd` listens on `$XDG_RUNTIME_DIR/chorus.sock` or `~/Library/Application Support/Chorus/chorusd.sock`. The native Swift UI communicates over this UDS using high-speed local IPC.
- **Local HTTP / WebSocket Server**: `chorusd` can bind to `127.0.0.1:4242` (with an optional authentication token). This allows local web frontends, CLI commands, and mesh tunnels (Tailscale) to communicate with Chorus.
- **Outbound Relay Connector (Phase 6 plug)**: An optional background task in `chorusd` that opens an outbound TLS connection to a relay, multiplexing the exact same internal API over the encrypted tunnel.

### 5.2 The Unified API Surface

The API surface maps 1:1 to Chorus core's ledger, session engine, diff engine, and memory subsystems:

#### A. Session Lifecycle & Configuration
- `POST /v1/sessions`: Create a new session.
  - Request: `{ project_path: string, initial_agent?: string, mode?: string }`
  - Response: `{ session_id: number, ledger_path: string }`
- `GET /v1/sessions`: List active and historical sessions.
- `GET /v1/sessions/{id}`: Get session summary, token usage, current state (`idle`, `busy`, `waiting_approval`).
- `POST /v1/sessions/{id}/fork`: Fork a session from a specific event sequence.
- `POST /v1/sessions/{id}/handoff`: Trigger an agent handoff (`codex` -> `claude`).

#### B. Turn Execution & Steering
- `POST /v1/sessions/{id}/turns`: Submit a prompt to run a turn.
  - Request:
    ```json
    {
      "agent": "codex",
      "prompt": "Refactor the database connection pool",
      "attachments": [12, 14],
      "model_override": "anthropic/claude-3-7-sonnet"
    }
    ```
  - Response: `{ turn_id: string, status: "started" }`
- `POST /v1/sessions/{id}/abort`: Interrupt/cancel the currently running turn.

#### C. Event Streaming (SSE & WebSocket)
- `GET /v1/sessions/{id}/events/stream?since={seq}`:
  - Standard Server-Sent Events (SSE) endpoint.
  - Streams every ledger event appended after sequence `seq`:
    - `user_message`
    - `agent_thought` (incremental text chunk)
    - `agent_message`
    - `tool_call_started`
    - `tool_call_finished`
    - `interaction_pending`
    - `interaction_resolved`
    - `turn_completed`

#### D. Asynchronous Interaction & Approval API
- `GET /v1/sessions/{id}/interactions`: List pending human-in-the-loop requests.
- `GET /v1/sessions/{id}/interactions/{interaction_id}`:
  - Response:
    ```json
    {
      "interaction_id": "act_98234",
      "session_id": 105,
      "tool": "bash",
      "kind": "execute",
      "detail": "cargo test --all",
      "risk_level": "medium",
      "created_at": "2026-10-09T04:25:00Z"
    }
    ```
- `POST /v1/sessions/{id}/interactions/{interaction_id}/resolve`:
  - Request:
    ```json
    {
      "decision": "allow", // "allow" | "deny" | "reply"
      "feedback": "Run only crates/chorus-core tests instead"
    }
    ```
  - Response: `{ ok: true }`

#### E. Diffs, Checkpoints & Hunk Rejections
- `GET /v1/sessions/{id}/diff`:
  - Response: Current working tree diff against turn baseline (unified diff + parsed hunks).
- `POST /v1/sessions/{id}/diff/reject-hunks`:
  - Request: `{ hunks: [{ file: "src/session.rs", hunk_index: 2 }] }`
  - Reverts specific hunks using Chorus's existing `apply_reverse` checkpoint logic.

#### F. Memory & Approve Queue
- `GET /v1/sessions/{id}/memory`: Inspect current memory state and pending propose queue.
- `POST /v1/sessions/{id}/memory/review`: Approve or reject proposed memory entries.

#### G. Webhook & Notification Registration
- `POST /v1/webhooks`: Register a webhook endpoint to receive notifications on events:
  - Events: `["interaction.created", "turn.completed", "agent.error"]`
  - Target: `https://my-relay-service.com/hooks` or `http://localhost:9090/notify`

---

## 6. Migration Roadmap from Today's C ABI to Headless Core

Chorus currently relies on `chorus-ffi` exporting blocking C functions to SwiftUI. To transition gracefully without breaking the macOS app:

```
[Phase Current]
SwiftUI App ----(In-Process C FFI)----> chorus-ffi ----> chorus-core / chorus-cli

[Phase 1-5 Target (Decoupled Daemon Layer)]
SwiftUI App ----(UDS / Localhost)----+
                                      |
Agent CLI / Tools --------------------+---> [chorusd HTTP/WS/SSE API] ---> chorus-core / session
                                      |
Mesh / Tailscale (Mobile) ------------+

[Phase 6 Future (Mobile Companion)]
Mobile App (iOS/Android) ---> [Encrypted Relay] ---> [chorusd Relay Connector] (Same API)
```

1. **Step 1: Replace Synchronous FFI Callbacks with Async Channels**:
   Update `chorus_cli::session::Session` so permission checks return a future waiting on a shared `InteractionManager` state machine rather than an immediate function pointer.
2. **Step 2: Implement Local HTTP/UDS Server (`chorusd`)**:
   Build a lightweight Axum or Actix-web server crate (`crates/chorus-server` or inside `chorus-cli`) exposing the endpoints specified in Section 5.2.
3. **Step 3: Point Native macOS Shell to Local IPC**:
   The Swift app can either talk directly to the UDS using `URLSession` / async streams, or maintain a thin FFI layer that forwards calls to the local engine.
4. **Step 4: Enable Mobile Access via Tailscale (Immediate Day-1 Win)**:
   Any user running Tailscale can hit the Chorus server from mobile Safari or a PWA immediately.
5. **Step 5: Add Push Notification Hooks**:
   Emit webhook payloads on `interaction.created` and `turn.completed` to trigger mobile alerts.
6. **Step 6: Build Zero-Knowledge Relay for Public Release (Phase 6)**:
   When building the dedicated mobile app, implement the Happy-style QR-code pairing and TweetNaCl encrypted relay without changing a single line of Chorus's core session logic.
