# Chat mode

Type: grilling
Status: resolved
Blocked by: 04

## Question

Decide plain chat:
- What a chat session is: no project, read-only, ledger-backed.
- The backends: agent CLIs in read-only mode plus local Ollama models, with a model picker.
- Where chats live relative to project sessions in the sidebar.
- Which chat-app features to include from the survey (branching, regenerate, compare, search).
- Whether the router applies to chat.
- How a chat is promoted into a project task with its context carried over.

## Answer

- **Chat Session Architecture**: A workspace-free, read-only session requiring no working directory and no shadow git initialization. Backed by the unified SQLite ledger (`ledger.rs`) as the single source of truth, enabling full event history and crash recovery. Filesystem-modifying tools are disabled.
- **Unified Sidebar Structure**: The macOS sidebar is divided into two distinct sections:
  - 💬 **Chats** (top section): Unbound brainstorming, quick Q&A, architectural ideation threads.
  - 📁 **Projects** (bottom section): Workspace directories and active code-modifying sessions.
- **Flexible Backends & Mid-Turn Switching**:
  - Supports both local Ollama models (`127.0.0.1:11434`, offline/private) and provider agent CLIs pinned to read-only mode (Codex, Claude Code, Gemini).
  - Users can switch models freely between turns in the same chat without losing context, as Chorus reassembles context dynamically from the ledger.
  - Includes a **Dual Split/Arena Mode** (`Cmd+T`) to query two models concurrently side-by-side using the `chorus compare` engine.
- **Model Selection & Optional Auto Routing**: Chat defaults to the user's explicitly pinned model (e.g. Claude 3.7 Sonnet or Ollama), with an optional "Auto" option in the dropdown picker for users who want the local router to choose per message.
- **Core Chat UX Features**:
  - Full branching / message editing via `ledger::fork`.
  - View-first **Artifacts Drawer** rendering syntax-highlighted code snippets, Markdown, SVG, and Mermaid diagrams.
  - Instant cross-chat full-text search powered by SQLite FTS5 (`Cmd+K`).
- **"Promote to Task" Workflow**: A dedicated toolbar button converts an exploratory conversation into an actionable project task. The user selects a target directory (`<cwd>`); Chorus synthesizes the conversation into an authoritative `handoff::Brief`, initializes shadow git checkpoints, and navigates seamlessly into the Workbench UI.
