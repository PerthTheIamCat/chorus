# Chat Application Survey & Analysis for Chorus

## Executive Summary

Chorus is designed as a desktop harness where multiple AI agents collaborate within a shared session, anchored by an immutable SQLite ledger (`crates/chorus-core`) and rendered through a native macOS SwiftUI shell (`apps/macos`). While Chorus's primary workspace mode is project-bound—equipped with shadow Git checkpoints (`checkpoint.rs`), git worktrees, file modification tracking, and multi-turn ACP bridge orchestration—users frequently require a lightweight **chat mode**: a zero-overhead, read-only scratchpad for quick questions, ideation, drafting, and architecture discussions before touching project code.

This survey examines seven leading conversational AI applications across the commercial cloud, open-source web, and local-first desktop ecosystems:
1. **ChatGPT Desktop App for macOS** (OpenAI)
2. **Claude Desktop App for macOS** (Anthropic)
3. **LibreChat** (Open-source multi-provider web/desktop client)
4. **T3 Chat** (Theo Browne / t3.chat fast multi-model aggregator)
5. **Msty** (Local-first + cloud multi-model desktop application)
6. **Open WebUI** (Feature-rich self-hosted interface for Ollama & APIs)
7. **LM Studio** (Local LLM runner, desktop playground, and developer CLI/server)

Each application is analyzed across eleven functional dimensions: model picker UX, message branching and editing, response regeneration, answer comparison, history search, projects and folders, attachments, artifacts/canvas, system prompts/personas, local model execution, and promotion into agentic/coding tasks.

Finally, this research evaluates how these patterns map to a **no-project, read-only Chorus session backed by agent CLIs (Codex, Claude, Gemini) and local Ollama**, adhering strictly to Chorus's architectural invariants: the ledger as single source of truth (`CLAUDE.md` §2.2), content-addressed deduplicated blob storage (`blob.rs`), view-first UI without embedded IDEs (`map.md`), and separation of storage from model context (`CLAUDE.md` §2.1).

---

## 1. Feature Comparison Matrix

| Dimension | ChatGPT macOS | Claude macOS | LibreChat | T3 Chat | Msty | Open WebUI | LM Studio |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **1. Model Picker UX** | Top dropdown; switch mid-chat; per-turn switch or retry | Top dropdown; locked per chat; start new chat to change | Top dropdown + Presets; switch mid-chat (re-sends ledger) | Composer selector; quick switch between top providers | Pinned selector; filter local vs. remote; per-pane picker | Multi-model selector; switch mid-chat; select multiple models | Top loader; select local GGUF; GPU layer sliders |
| **2. Branching & Editing** | Inline edit creates branch (`< 1/N >`); "Branch in new chat" | Inline edit creates branch with `< 1/N >` pagination | Inline edit (`Cmd+Enter`); Fork menu with 3 scope modes | Inline edit; variation creation | Visual Branch Explorer; fork from any message; export tree | Inline edit; `/fork` slash command; `< 1/N >` controls | Inline edit; response pagination; API `response_id` fork |
| **3. Regenerate** | Retry button under response; pick alternate model | Retry button under response (same model) | Regenerate button; retry with same or altered preset | Regenerate button; retry with same or different model | Rerun button; adjust temperature / top_p on retry | Regenerate button; re-query model or switch model | Regenerate button; adjust sampler settings per retry |
| **4. Compare Answers** | Occasional automated RLHF prompt evaluation only | None natively | Compare presets / manual fork comparison | Sequential multi-model queries on same prompt | **Split Chat** (`Cmd+T`): side-by-side sync typing | **Compare Mode**: parallel columns or stacked streams | **Playground**: side-by-side local model evaluation |
| **5. Search History** | Full-text sidebar search (`Cmd+K`); filters by type | Full-text sidebar search across past chats | **Meilisearch** (`SEARCH=true`): instant typo-tolerant FTS | Fast client-side search across IndexedDB cache | Advanced Search: filter by text, date, model, tags | Full-text search (`Cmd+K`); titles, messages, tags | Sidebar search bar filtering chat titles and snippets |
| **6. Projects & Folders** | Projects (Pro/Team); custom instructions + files | Projects (Pro/Team); 200k docs + instructions | Bookmarks, preset groups, conversation tags | Flat thread list; search-focused | Folders, tags, Knowledge Stacks (per-folder RAG) | Folders, tags, workspace collections, pinned chats | Chat organization folders; saved configuration presets |
| **7. Attachments** | Files, images, PDFs, code; Python sandbox analysis | Images, PDFs, text files (up to 5 / 30MB per message) | Multi-file upload; vector embeddings RAG; Code Sandbox | PDFs, images, text files for context grounding | Documents (PDF, DOCX, TXT); folder RAG; Vision GGUFs | Universal upload (doc/image/audio); auto-RAG; web URLs | Vision images (Llama-3.2-Vision); Chat with Documents |
| **8. Artifacts & Canvas** | **Canvas**: side-by-side document/code editor + diffs | **Artifacts**: side pane; React, SVG, Mermaid, HTML | **Artifacts**: side panel; React, HTML, Mermaid, SVG | Syntax-highlighted code blocks, copy, KaTeX | Syntax-highlighted code; side panels for RAG/Split | **Artifacts**: dedicated window; HTML/React/SVG, versions | Syntax-highlighted code blocks; token inspector pane |
| **9. System Prompts & Personas** | Custom Instructions; Custom GPTs; Project Prompts | Project system instructions; preset Style picker | Presets (prompt + params); custom LibreChat Agents | Personas (saved prompt + preferred model pairings) | Custom Personas (instructions, temperature, model) | Modelfiles; Workspace custom models; system prompts | Config Presets (prompt, Jinja chat templates, samplers) |
| **10. Local-Model Support** | None (Cloud OpenAI API only) | None (Cloud Anthropic API only) | First-class Ollama, LocalAI, LM Studio, vLLM HTTP | None (Cloud aggregator API only) | Native built-in llama.cpp + auto-detected Ollama | Native first-class Ollama; OpenAI-compatible backends | Core competency: native llama.cpp + Metal + HF Hub |
| **11. Agentic / Coding Promotion** | **Work with Apps** (macOS Accessibility); Codex app | **MCP** (`claude_desktop_config.json`); Cowork | **Agents API**, subagents, tool-calling, MCP servers | External handoff / clipboard; T3 Code sister app | **Msty Studio v2**: MCP tool integration, workflows | Tools / Functions, MCP servers, Code Interpreter | Headless local server (`lms local`); Bionic code agent |

---

## 2. In-Depth Profiles of Surveyed Applications

### 2.1 ChatGPT Desktop App for macOS
* **Primary Sources:**
  - OpenAI Help Center: *Using the ChatGPT desktop app for macOS* ([help.openai.com/en/articles/9275200](https://help.openai.com/en/articles/9275200-using-the-chatgpt-desktop-app-for-macos))
  - OpenAI Help Center: *Search your chat history* ([help.openai.com/en/articles/9930777](https://help.openai.com/en/articles/9930777-search-your-chat-history))
  - OpenAI Help Center: *Work with Apps on macOS* ([help.openai.com/en/articles/10119602](https://help.openai.com/en/articles/10119602-work-with-apps-on-macos))
  - OpenAI Product Announcement: *Introducing Canvas* ([openai.com/index/canvas](https://openai.com/index/canvas/))
  - OpenAI Help Center: *Managing your ChatGPT conversations & branching* ([help.openai.com/en/articles/8555545](https://help.openai.com/en/articles/8555545-managing-your-chatgpt-conversations))
* **UX & Interaction Mechanics:**
  - **Model Picker:** Positioned in the header bar. Users can toggle between standard reasoning models (`GPT-4o`, `o1`, `o3-mini`, `GPT-4o mini`). Switching models mid-conversation applies to the next turn; users can also regenerate a specific turn with a different model.
  - **Branching & Editing:** Hovering on a user message displays a pencil icon. Submitting an edit forks the conversation tree at that point, replacing downstream visible messages while retaining parent branches. Navigation between alternate branches is exposed via `< 1/N >` pagination buttons below the message. Additionally, a dedicated "Branch in a new chat" menu option clones conversation history up to the selected assistant turn into a separate sidebar thread.
  - **Search:** Instant global search accessible via `Cmd+K` or the sidebar magnifying glass. Performs full-text indexing over message text, conversation titles, custom GPTs, and attached document metadata.
  - **Canvas:** Rather than dumping large code files or documents into the conversation stream, Canvas opens a synchronized two-pane layout. The right-hand pane acts as a live editor where the user can highlight text, trigger inline edits, inspect diffs, and slide through revision history.
  - **Agentic Promotion ("Work with Apps"):** Leveraging macOS Accessibility permissions and AppleScript/System Events, the desktop app inspects active windows in developer tools (Xcode, VS Code, JetBrains IDEs, Terminal, iTerm2, Warp, Notes). When promoted, ChatGPT generates context-aware diffs and applies code modifications directly to local workspace files upon user approval.

### 2.2 Claude Desktop App for macOS
* **Primary Sources:**
  - Anthropic Support: *Claude for Desktop Overview* ([support.anthropic.com/en/articles/9986348](https://support.anthropic.com/en/articles/9986348-claude-for-desktop))
  - Anthropic Support: *What are Artifacts and how do I use them?* ([support.anthropic.com/en/articles/9487310](https://support.anthropic.com/en/articles/9487310-what-are-artifacts-and-how-do-i-use-them))
  - Anthropic Support: *Using Projects in Claude* ([support.anthropic.com/en/articles/9517075](https://support.anthropic.com/en/articles/9517075-what-are-projects))
  - Model Context Protocol Documentation: *Quickstart for Desktop Users* ([modelcontextprotocol.io/quickstart/user](https://modelcontextprotocol.io/quickstart/user))
* **UX & Interaction Mechanics:**
  - **Model Picker:** Located above the composer. Allows selection among `Claude 3.7 Sonnet`, `Claude 3.5 Haiku`, and `Claude 3 Opus`. Model selection is locked once a conversation begins; switching models requires launching a new conversation.
  - **Branching & Editing:** Users can edit prior prompts. An edit forks the transcript from that point forward and displays `< 1/N >` buttons to step across historical branches.
  - **Artifacts:** When Claude generates self-contained code (HTML, React, SVG, Mermaid, Python, Markdown), it isolates the content into a dedicated right-hand Artifacts drawer. The drawer offers a Code/Preview toggle (running interactive React components in a sandboxed iframe), a version history slider, and direct copy/download buttons.
  - **Projects:** Users can group related conversations into "Projects" with up to 200,000 tokens of persistent project reference documents and custom system instructions.
  - **Agentic Promotion (Model Context Protocol):** Claude Desktop serves as a primary MCP client host. Configured via `~/Library/Application Support/Claude/claude_desktop_config.json`, local `stdio` subprocesses expose tools, resources, and prompts directly to Claude. This allows Claude to query local databases, inspect local file systems, and execute commands while remaining within a chat UI.

### 2.3 LibreChat
* **Primary Sources:**
  - LibreChat Documentation: *Features Overview* ([docs.librechat.ai/features/](https://docs.librechat.ai/features/))
  - LibreChat Documentation: *Conversation Forking* ([docs.librechat.ai/features/fork.html](https://docs.librechat.ai/features/fork.html))
  - LibreChat Documentation: *Message Actions* ([docs.librechat.ai/features/message_actions.html](https://docs.librechat.ai/features/message_actions.html))
  - LibreChat Documentation: *Artifacts (Generative UI)* ([docs.librechat.ai/features/artifacts.html](https://docs.librechat.ai/features/artifacts.html))
  - LibreChat Documentation: *Model Context Protocol (MCP)* ([docs.librechat.ai/features/mcp.html](https://docs.librechat.ai/features/mcp.html))
  - LibreChat Documentation: *Meilisearch Integration* ([docs.librechat.ai/features/meilisearch.html](https://docs.librechat.ai/features/meilisearch.html))
* **UX & Interaction Mechanics:**
  - **Model Picker & Presets:** Unified header selector spanning cloud APIs (OpenAI, Anthropic, Google, Mistral) and local endpoints (Ollama, LocalAI). Presets bundle an endpoint, model name, system instructions, temperature, and context settings into a 1-click template.
  - **Branching Mechanics:** The most flexible branching system in open-source chat. Hovering over a turn reveals a Fork icon with three granular fork modes:
    1. *Visible messages only:* Copies strictly the active path leading to the target message.
    2. *Include related branches:* Copies the active path plus sibling branches on that path.
    3. *Include all to/from here:* Clones the full DAG surrounding the message.
    Inline message editing supports `Cmd+Enter` to save and rerun, or `Cmd+S` to edit history retroactively without triggering a new inference turn.
  - **Search:** Backed by an optional Meilisearch service container (`SEARCH=true`). Delivers sub-millisecond typo-tolerant full-text search across titles and all raw message bodies.
  - **Artifacts:** Supports full Generative UI rendering (React/shadcn, HTML, SVG, Mermaid diagrams) within a collapsible right pane with full-screen and pop-out window capabilities.
  - **Agentic Promotion:** LibreChat Agents support tool calling, file sandboxing, and subagent orchestration. Configured in `librechat.yaml`, agents can invoke external MCP servers or run Python code in an isolated sandbox.

### 2.4 T3 Chat
* **Primary Sources:**
  - T3 Chat Official Site & Feedback Portal: ([t3.chat](https://t3.chat/); [feedback.t3.chat](https://feedback.t3.chat/))
  - T3 Open Source Organization: ([github.com/t3-oss](https://github.com/t3-oss))
  - Startup Intros Platform Profile: *T3 Chat Overview* ([startupintros.com/company/t3-chat](https://startupintros.com/company/t3-chat))
* **UX & Interaction Mechanics:**
  - **Model Picker:** Focuses on ultra-fast switching across top-tier models (OpenAI GPT-4o, Anthropic Claude 3.5/3.7, Google Gemini 2.0, DeepSeek R1/V3) within the composer.
  - **Performance Architecture:** Employs a local-first client architecture using IndexedDB for local chat caching, paired with Cloudflare edge routing to minimize time-to-first-token.
  - **Comparison & Iteration:** Emphasizes rapid prompt iteration. Users can query one model, evaluate the response, and immediately query another model on the same problem.
  - **Attachments & Personas:** Lightweight attachment support for PDFs, text files, and images for prompt grounding. Personas allow saving custom system instructions tied to preferred models.
  - **Agentic Positioning:** T3 Chat maintains a strict boundary as a pure conversational interface. For agentic software engineering, Theo Browne’s ecosystem bifurcates into **T3 Code**, an experimental CLI/agent harness designed specifically for codebase editing.

### 2.5 Msty
* **Primary Sources:**
  - Msty Official Portal: ([msty.app](https://msty.app/))
  - Msty Documentation: *Split Chat Guide* ([msty.app/docs/chat/split-chat](https://msty.app/docs/chat/split-chat))
  - Msty Documentation: *Branching & Branch Explorer* ([msty.app/docs/chat/branching](https://msty.app/docs/chat/branching))
  - Msty Documentation: *Offline Models & Local Engine* ([msty.app/docs/models/offline-models](https://msty.app/docs/models/offline-models))
  - Msty Documentation: *Using Existing Ollama Models* ([docs.msty.app/docs/models/use-existing-ollama-models](https://docs.msty.app/docs/models/use-existing-ollama-models))
  - Msty Documentation: *Advanced Search* ([docs.msty.app/docs/chat/advanced-search](https://docs.msty.app/docs/chat/advanced-search))
* **UX & Interaction Mechanics:**
  - **Model Architecture:** Hybrid local/cloud desktop application. Embedded llama.cpp engine with hardware acceleration (Metal on Apple Silicon) alongside direct integration with Ollama (auto-detects local Ollama installations and model weights).
  - **Split Chat (Answer Comparison):** Msty's signature feature. Pressing `Cmd+T` (or clicking Split) splits the active window into side-by-side chat columns. In Duplication Mode, text typed into the shared composer broadcasts concurrently to 2–4 models, displaying streaming outputs side-by-side for real-time comparison. Split configurations can be saved as reusable "Split Presets".
  - **Branch Explorer:** Instead of rudimentary `< 1/N >` pagination arrows, Msty provides a visual "Branch Explorer" drawer. Users can view the conversation tree, rename branches, prune dead ends, and switch branches instantly. Aurum/Pro users can export the full DAG or cherry-pick specific turns.
  - **Knowledge Stacks:** File attachments can be organized into reusable "Knowledge Stacks" (local folders of PDFs, technical manuals, or codebases) that provide local RAG context across any chat session.
  - **Agentic Promotion:** The newer Msty Studio (v2.0) introduces local MCP tool support, custom workflow builders, and "Bionic" agent mode for local code editing.

### 2.6 Open WebUI
* **Primary Sources:**
  - Open WebUI Documentation: *Chat & Multi-Model Features* ([docs.openwebui.com/features/chat/](https://docs.openwebui.com/features/chat/))
  - Open WebUI Documentation: *Artifacts Guide* ([docs.openwebui.com/features/chat/artifacts/](https://docs.openwebui.com/features/chat/artifacts/))
  - Open WebUI Documentation: *Workspace Models & Modelfiles* ([docs.openwebui.com/features/workspace/models/](https://docs.openwebui.com/features/workspace/models/))
  - Open WebUI Documentation: *Tools & External Functions* ([docs.openwebui.com/features/workspace/tools/](https://docs.openwebui.com/features/workspace/tools/))
  - Open WebUI Documentation: *Quickstart & URL Parameters* ([docs.openwebui.com/getting-started/quickstart/#url-parameters](https://docs.openwebui.com/getting-started/quickstart/#url-parameters))
* **UX & Interaction Mechanics:**
  - **Multi-Model Compare Mode:** The model picker allows selecting multiple models simultaneously (or launching via URL `/?models=llama3.2,qwen2.5:3b`). Streaming responses appear side-by-side in parallel columns (or stacked), allowing users to compare reasoning speed and output quality across local and remote models on a single prompt.
  - **Model Customization (Modelfiles):** Direct integration with Ollama allows users to create and edit custom models using the Ollama Modelfile syntax (specifying base weights, system prompts, stop tokens, and parameters like temperature).
  - **Artifacts:** Comprehensive generative UI support. Renders HTML, SVG, Markdown, and React components within a sandboxed side panel. Includes a built-in syntax-highlighted code editor, diff comparison across generated versions, and full-screen preview.
  - **Branching & Editing:** Inline message editing automatically spawns conversation branches with `< 1/N >` turn navigation controls. A `/fork` slash command allows branching from any point.
  - **Search & Workspace Organization:** Global search (`Cmd+K`) covers conversation titles, message contents, and tags. Conversations can be organized into nested folders, pinned to the top, or filtered by tags.
  - **Agentic Capabilities:** Extensive tool framework supporting Python Functions/Pipelines, OpenAPI tool servers, external MCP servers, and an integrated sandboxed Code Interpreter.

### 2.7 LM Studio
* **Primary Sources:**
  - LM Studio Documentation: *Basics & Managing Chats* ([lmstudio.ai/docs/app/basics/chat](https://lmstudio.ai/docs/app/basics/chat))
  - LM Studio Documentation: *Config Presets & Hub* ([lmstudio.ai/docs/app/presets](https://lmstudio.ai/docs/app/presets))
  - LM Studio Documentation: *Prompt Templates & Jinja* ([lmstudio.ai/docs/advanced/prompt-template](https://lmstudio.ai/docs/advanced/prompt-template))
  - LM Studio Documentation: *Local REST API & Chat Completions* ([lmstudio.ai/docs/developer/rest/chat](https://lmstudio.ai/docs/developer/rest/chat))
  - LM Studio Documentation: *CLI `lms` Tooling* ([lmstudio.ai/docs/cli/local-models/chat](https://lmstudio.ai/docs/cli/local-models/chat))
* **UX & Interaction Mechanics:**
  - **Model Engine & Picker:** Built entirely around local GGUF models running via llama.cpp. Top bar loader manages model downloads from Hugging Face, GPU layer offloading (Apple Metal), context window allocation, and CPU thread counts.
  - **Playground (Multi-Model Comparison):** Dedicated Playground tab allows loading multiple models into unified memory simultaneously. A single user prompt is dispatched to all loaded models, streaming responses side-by-side with real-time tokens-per-second, time-to-first-token, and total VRAM metrics.
  - **Branching & History:** Supports editing any user or assistant turn. The REST API (`/api/v1/chat`) explicitly accepts a `response_id` parameter to fork conversations at arbitrary turns without state loss.
  - **Presets & Chat Templates:** Allows saving fine-grained "Config Presets" that bundle the model's system prompt, Jinja chat template, temperature, top-p, min-p, repeat penalty, and max output tokens. Presets can be published and downloaded through the LM Studio Hub.
  - **Developer Promotion:** Built-in local server (`lms server start` on `localhost:1234`) exposes OpenAI- and Anthropic-compatible endpoints. This allows external tools (Cursor, Aider, Chorus) to consume LM Studio models programmatically. Additionally, the desktop app includes "Bionic", an agent mode capable of inspecting and editing local codebases.

---

## 3. Fit for Chorus: No-Project, Read-Only Chat Mode

In Chorus, a project session is heavily instrumented: it binds to a repository working directory (`cwd`), provisions a shadow Git repository under `<cwd>/.chorus/ckpt` (`checkpoint.rs`), tracks dirty working tree diffs, watches for file modifications, and runs ACP agent bridges with write permissions.

In contrast, a **Chorus Chat Mode session** is defined as:
* **No Project (`cwd = None`):** No association with a local repository or filesystem directory.
* **Read-Only (`tools = read_only` or mock tools):** ACP bridges are launched in read-only mode (`--read-only`), preventing arbitrary shell execution or filesystem mutations.
* **Dual Backend Support:** Seamlessly alternates between local **Ollama** models (fast, free, offline) and existing **Agent CLIs** (Codex via `codex-acp`, Claude via `@zed-industries/claude-agent-acp`, Gemini via `gemini --acp`).
* **Ledger-Backed Single Source of Truth:** Every turn, model switch, branch, and attachment is recorded into the SQLite ledger (`crates/chorus-core`).

Below is an architectural fit analysis for each surveyed feature category within Chorus:

```
+------------------------------------------------------------------------------------+
|                               CHORUS CHAT SESSION                                  |
|                                                                                    |
|  +------------------------+  +------------------------+  +----------------------+  |
|  |     Model Picker       |  |  Branching & Compare   |  |  View-First Artifact |  |
|  | (Ollama / Codex / etc) |  |   (Ledger DAG Forks)   |  |  (Markdown/HTML/SVG) |  |
|  +-----------+------------+  +-----------+------------+  +----------+-----------+  |
|              |                           |                          |              |
|              v                           v                          v              |
|  +------------------------------------------------------------------------------+  |
|  |                     Chorus Core Ledger (SQLite WAL)                          |  |
|  |           events: append / fork / history() | blob_store: dedup BLAKE3       |  |
|  +---------------------------------------+--------------------------------------+  |
|                                          |                                         |
|                   [ User decides to convert to code task ]                         |
|                                          |                                         |
|                                          v                                         |
|                     +-----------------------------------------+                    |
|                     |        "Promote to Task" Action         |                    |
|                     | - Select workspace folder (<cwd>)       |                    |
|                     | - Assemble ledger Brief (handoff.rs)    |                    |
|                     | - Initialize shadow Git (.chorus/ckpt)  |                    |
|                     | - Enable write tools & worktrees        |                    |
|                     +-----------------------------------------+                    |
|                                          |                                         |
|                                          v                                         |
|  +------------------------------------------------------------------------------+  |
|  |                         CHORUS WORKBENCH SESSION                             |  |
|  |  Multi-Agent Orchestration · Git Worktrees · Diff Review Loop · File Viewer  |  |
|  +------------------------------------------------------------------------------+  |
+------------------------------------------------------------------------------------+
```

### 3.1 Model Picker UX: Native to Chorus Invariants
* **Fit:** **High / Essential**
* **Architectural Rationale:**
  Because the session belongs to Chorus's ledger and *not* to a remote provider session (`CLAUDE.md` §2.2), Chorus is inherently capable of switching models mid-conversation.
  - When switching from `ollama/qwen2.5:3b` on Turn 1 to `codex` on Turn 2, Chorus's Rust `Session::assemble_context` gathers the history from the ledger, formats the prompt for the new target agent/model, and dispatches the turn.
  - In the SwiftUI shell, the model picker should reside in the composer bar:
    - A dropdown listing configured backends: Local Ollama models (detected via `GET http://localhost:11434/api/tags`), Codex (`codex-acp`), Claude (`@zed-industries/claude-agent-acp`), and Gemini (`gemini --acp`).
    - Every turn records its executing `actor` (agent/model identity) in the ledger event header, maintaining complete provenance.

### 3.2 Branching & Editing Messages: Direct Map to Ledger DAG
* **Fit:** **High / Native to Core Engine**
* **Architectural Rationale:**
  `crates/chorus-core/src/ledger.rs` already models sessions as a directed acyclic graph (DAG) of events, featuring:
  - `ledger::fork(parent_event_id)`: creates a new branch pointer with zero data copying.
  - `ledger::history(head_event_id)`: traverses backwards along the parent chain to reconstruct the exact transcript of any branch.
  - Dedup blob store (`blob.rs`): alternate branches that reuse text or file attachments incur zero storage overhead due to BLAKE3 content-addressing (`CLAUDE.md` §2.1).
  - **UI Implementation:** When a user edits a prior turn in chat mode, Chorus calls `ledger::fork`, appends the revised user turn, and streams the new response. The Swift UI presents standard `< 1/N >` pagination arrows at the edited turn.

### 3.3 Regenerate & Compare Answers (Dual / Arena)
* **Fit:** **High / Leverage Existing Compare Engine**
* **Architectural Rationale:**
  Chorus already implements response comparison in its CLI and macOS UI:
  - CLI: `chorus compare "<prompt>" --db /tmp/s.db --session 1 --agents codex,claude-opus` (`CLAUDE.md` §4).
  - FFI / macOS: Compare sheet and `chorus_run_compare`.
  - In Chat Mode, comparing answers (mirroring Msty's Split Chat or Open WebUI's Compare mode) is a natural extension:
    - User can check a "Compare" toggle or click "Regenerate with another model".
    - Chorus forks the ledger into two parallel branches, runs both models concurrently, and displays streaming responses in a two-column split view.
    - The user can select the preferred response to become the active conversation branch, leaving the alternate branch stored in the ledger for future reference.

### 3.4 Search Across History: SQLite FTS5 Already Implemented
* **Fit:** **High / Zero Dependency Addition**
* **Architectural Rationale:**
  Unlike LibreChat which requires deploying a separate Meilisearch service container, Chorus already includes full-text search directly inside SQLite via `FTS5` (`crates/chorus-core/src/schema.rs`).
  - `schema.rs` provisions an FTS5 virtual table indexing message bodies and metadata, with automatic fallback when FTS5 is compiled out.
  - A global `Cmd+K` palette in the macOS app can query `ledger::search_fts` across all chat and project sessions instantly with zero network or memory overhead.

### 3.5 Projects & Folders: Keep Chat Mode Flat & Uncluttered
* **Fit:** **Low for Folders / High for Sidebar Separation**
* **Architectural Rationale:**
  Chat mode exists precisely because users want to avoid project overhead.
  - In the macOS sidebar, Chat sessions should be segregated into a top-level **"Chats"** or **"Quick Chats"** section, separate from the **"Projects / Workspaces"** section.
  - Introducing deep folder hierarchies for chats adds unnecessary complexity. A flat list sorted by recent activity, complemented by tags and FTS5 search, matches the lightweight nature of desktop tools like T3 Chat and LM Studio.

### 3.6 Attachments: Read-Only Ingestion via Content-Addressed Blobs
* **Fit:** **Moderate / Read-Only Context Ingestion Only**
* **Architectural Rationale:**
  In project sessions, `@file` mentions resolve against the workspace filesystem. In a no-project chat session:
  - Users drag-and-drop images, paste screenshots, or drop text/PDF documents into the composer.
  - The content is hashed and stored in Chorus's blob store (`blob.rs`), returning a `BlobId`.
  - For image attachments: If the active model supports vision (e.g., Claude ACP or local `llama3.2-vision`), the blob is encoded as a base64 image content block (`01-attachment-compatibility.md`).
  - For text/PDF attachments: Chorus extracts the plain text and inserts it into the prompt assembly ladder (`rung L0/L1`), without writing any files to disk.

### 3.7 Artifacts & Canvas: View-First Side Panel
* **Fit:** **High / Strictly Adheres to View-First Policy**
* **Architectural Rationale:**
  Chorus explicitly forbids in-app file editing and IDE features (`map.md`: *"File handling is view-first... No in-app editing, no IDE"*).
  - Claude and LibreChat's **Artifacts** pattern is a perfect match: it is *view-first*.
  - When a model outputs code, SVG diagrams, Mermaid flowcharts, or markdown documents, Chorus detects the block and renders it in a dedicated collapsible right-hand "Artifact Viewer" pane.
  - The pane includes syntax highlighting, a copy button, and an "Open in External Editor" action, keeping Chorus focused on presentation rather than code editing.

### 3.8 System Prompts & Personas: User-Level Presets
* **Fit:** **Moderate / User Configuration Scope**
* **Architectural Rationale:**
  Because there is no project folder to host `.chorus/skills/` or `CLAUDE.md`, personas in chat mode should live in the global application configuration (`~/.chorus/config.json` or macOS Application Support).
  - Users can define lightweight presets: Name, System Prompt, and Preferred Model (e.g., "Architecture Reviewer" using Claude 3.7 Sonnet, "Quick Refactor" using Codex, "Offline Brainstorm" using Ollama Qwen 2.5).

### 3.9 Local-Model Support: Ollama as a First-Class Pillar
* **Fit:** **High / Core Synergy**
* **Architectural Rationale:**
  Chorus already integrates Ollama for background memory compaction (`compaction.rs: Ollama`, `CHORUS_SUMMARY_MODEL`).
  - By adding an `OllamaAgent` turn runner alongside `CodexAgent`, Chorus allows completely free, private, offline conversations without API keys or CLI bridge logins.
  - The engine polls `http://localhost:11434/api/tags` to populate available local models in the picker.

---

## 4. The Promotion Bridge: Elevating Chat to an Agentic Project Task

The most critical strategic differentiator for Chorus is the transition from **Plain Chat** to **Agentic Workspace Task**.

In apps like ChatGPT, Claude, or Open WebUI, a conversation either remains a chat forever or relies on loose copy-pasting into an IDE. In Chorus, a chat session can be directly **promoted** into an autonomous engineering task with complete context preservation.

### 4.1 Promotion Flow & Mechanics

```
Chat Session (Read-Only, No Project)
   │
   │  [User clicks "Promote to Task" or types /promote]
   ▼
Promotion Modal / Sheet
   ├── 1. Select Workspace Folder: /Users/perth/private_project/chorus
   ├── 2. Select Execution Agent: Codex / Claude / Gemini
   ├── 3. Context Distillation Options:
   │       ├── [A] Full Transcript: Carry all turns into project ledger
   │       └── [B] Structured Brief: Run Extractive/Ollama summarizer
   ▼
New Project Session Initialized
   ├── 1. Initialize Shadow Git: <cwd>/.chorus/ckpt snapshot
   ├── 2. Populate Context: Brief injected as Turn 0 context
   ├── 3. Grant Permissions: Enable write tools, worktrees, diff tracking
   └── 4. Launch Agent Bridge: Agent begins executing code modifications
```

1. **Trigger:** The user clicks the **"Promote to Task"** button in the chat header (or invokes `/promote`).
2. **Target Association:** A native macOS open-panel prompts the user to select the target workspace directory (`cwd`).
3. **Context Distillation:**
   Rather than blindly dumping hundreds of exploratory conversational tokens into the coding agent's active context window (`CLAUDE.md` §2.1: *storage ≠ context*), Chorus offers two transfer strategies:
   - **Structured Brief (Recommended):** Chorus executes its built-in summarizer (`crates/chorus-core/src/compaction.rs` via Extractive or Ollama) to synthesize the chat into a `Brief` (`handoff.rs`). The brief captures:
     - **Goal:** The final decision or architecture settled on during chat.
     - **Key Decisions:** Constraints and rejected alternatives discussed.
     - **Target Files:** File paths mentioned in conversation.
   - **Full Transcript Fork:** The project session forks directly from the chat ledger's tail event, bringing all raw conversation turns into the project ledger.
4. **Environment Escalation:**
   - A shadow git checkpoint (`checkpoint.rs`) is initialized on `<cwd>`.
   - The session transitions from read-only mode to full workbench mode: agent tools are enabled, per-hunk diff tracking is activated, and parallel agents receive dedicated git worktrees (`map.md` §Notes).
   - The user selects the executing agent (e.g., Codex) and types the initial action prompt.

---

## 5. Architectural Recommendations for Issue 18 (Chat Mode)

Based on this survey, the following decisions are recommended for the upcoming grilling ticket `issues/18-chat-mode.md`:

1. **Session Schema & Storage:**
   - Chat sessions use the exact same SQLite ledger schema (`schema.rs` v3) as project sessions.
   - A chat session is identified simply by `cwd: NULL` (or empty string) and `mode: "chat"` in session metadata.
   - Stored in a global chat database (`~/.chorus/chat.db`) or the unified database with a session-level workspace flag.

2. **Model Picker & Backends:**
   - Primary backends:
     1. **Local Ollama:** Direct HTTP client to `localhost:11434` (zero external processes required).
     2. **Agent CLIs in Read-Only Mode:** Codex, Claude, and Gemini ACP bridges invoked with flags disallowing file modifications (`--read-only` or empty tool manifests).
   - Support switching models mid-conversation on every turn. The ledger remains the source of truth, assembling context dynamically per target model.

3. **Router Application:**
   - **Recommendation: Do NOT run the automatic router by default on plain chat.**
   - Chat is an intentional, interactive user experience. Users explicitly choose whether they want a fast, free local Ollama model or a high-capability cloud model.
   - However, an optional **"Auto (Budget)"** selector can be provided, which biases toward local Ollama and cheap models per `map.md` standing preferences.

4. **UI Layout in macOS Shell:**
   - **Sidebar:** Place a dedicated **"Chats"** section at the top of the sidebar, above workspace projects.
   - **Compare View:** Support a side-by-side split view triggered by a "Compare" button, executing concurrent turns across two selected models via ledger forks.
   - **Artifacts:** Adopt a view-first right-hand drawer that renders code blocks, Markdown, and SVG/Mermaid diagrams with syntax highlighting and an "Open in External Editor" button.

5. **Promotion Action:**
   - Add a prominent **"Promote to Task"** button in the window toolbar.
   - Use `Brief::build` from `crates/chorus-core/src/handoff.rs` to distill conversation history into a structured brief before initializing the project session.

---

## 6. Primary Sources & Citations

- **OpenAI ChatGPT Desktop for macOS:**
  - *Using the ChatGPT desktop app for macOS:* [https://help.openai.com/en/articles/9275200-using-the-chatgpt-desktop-app-for-macos](https://help.openai.com/en/articles/9275200-using-the-chatgpt-desktop-app-for-macos)
  - *Search your chat history:* [https://help.openai.com/en/articles/9930777-search-your-chat-history](https://help.openai.com/en/articles/9930777-search-your-chat-history)
  - *Work with Apps on macOS:* [https://help.openai.com/en/articles/10119602-work-with-apps-on-macos](https://help.openai.com/en/articles/10119602-work-with-apps-on-macos)
  - *Introducing Canvas:* [https://openai.com/index/canvas/](https://openai.com/index/canvas/)
  - *Managing your ChatGPT conversations & branching:* [https://help.openai.com/en/articles/8555545-managing-your-chatgpt-conversations](https://help.openai.com/en/articles/8555545-managing-your-chatgpt-conversations)
- **Anthropic Claude Desktop for macOS:**
  - *Claude for Desktop Overview:* [https://support.anthropic.com/en/articles/9986348-claude-for-desktop](https://support.anthropic.com/en/articles/9986348-claude-for-desktop)
  - *What are Artifacts and how do I use them?:* [https://support.anthropic.com/en/articles/9487310-what-are-artifacts-and-how-do-i-use-them](https://support.anthropic.com/en/articles/9487310-what-are-artifacts-and-how-do-i-use-them)
  - *Using Projects in Claude:* [https://support.anthropic.com/en/articles/9517075-what-are-projects](https://support.anthropic.com/en/articles/9517075-what-are-projects)
  - *Model Context Protocol Desktop Quickstart:* [https://modelcontextprotocol.io/quickstart/user](https://modelcontextprotocol.io/quickstart/user)
- **LibreChat:**
  - *LibreChat Documentation & Features:* [https://docs.librechat.ai/features/](https://docs.librechat.ai/features/)
  - *Conversation Forking:* [https://docs.librechat.ai/features/fork.html](https://docs.librechat.ai/features/fork.html)
  - *Message Actions:* [https://docs.librechat.ai/features/message_actions.html](https://docs.librechat.ai/features/message_actions.html)
  - *Artifacts (Generative UI):* [https://docs.librechat.ai/features/artifacts.html](https://docs.librechat.ai/features/artifacts.html)
  - *Model Context Protocol (MCP):* [https://docs.librechat.ai/features/mcp.html](https://docs.librechat.ai/features/mcp.html)
  - *Meilisearch Integration:* [https://docs.librechat.ai/features/meilisearch.html](https://docs.librechat.ai/features/meilisearch.html)
- **T3 Chat:**
  - *T3 Chat Web Application & Feedback:* [https://t3.chat/](https://t3.chat/) · [https://feedback.t3.chat/](https://feedback.t3.chat/)
  - *T3 Open Source Organization:* [https://github.com/t3-oss](https://github.com/t3-oss)
  - *T3 Chat Overview:* [https://startupintros.com/company/t3-chat](https://startupintros.com/company/t3-chat)
- **Msty:**
  - *Msty Official Documentation:* [https://docs.msty.app/](https://docs.msty.app/)
  - *Split Chat & Duplication Mode:* [https://msty.app/docs/chat/split-chat](https://msty.app/docs/chat/split-chat)
  - *Branching & Branch Explorer:* [https://msty.app/docs/chat/branching](https://msty.app/docs/chat/branching)
  - *Offline Models & Local Engine:* [https://msty.app/docs/models/offline-models](https://msty.app/docs/models/offline-models)
  - *Use Existing Ollama Models:* [https://docs.msty.app/docs/models/use-existing-ollama-models](https://docs.msty.app/docs/models/use-existing-ollama-models)
  - *Advanced Search:* [https://docs.msty.app/docs/chat/advanced-search](https://docs.msty.app/docs/chat/advanced-search)
- **Open WebUI:**
  - *Open WebUI Documentation & Multi-Model Chat:* [https://docs.openwebui.com/features/chat/](https://docs.openwebui.com/features/chat/)
  - *Artifacts System:* [https://docs.openwebui.com/features/chat/artifacts/](https://docs.openwebui.com/features/chat/artifacts/)
  - *Workspace Models & Modelfiles:* [https://docs.openwebui.com/features/workspace/models/](https://docs.openwebui.com/features/workspace/models/)
  - *Tools & Tool Servers:* [https://docs.openwebui.com/features/workspace/tools/](https://docs.openwebui.com/features/workspace/tools/)
  - *Quickstart & URL Parameters:* [https://docs.openwebui.com/getting-started/quickstart/#url-parameters](https://docs.openwebui.com/getting-started/quickstart/#url-parameters)
- **LM Studio:**
  - *LM Studio Documentation Basics:* [https://lmstudio.ai/docs/app/basics/chat](https://lmstudio.ai/docs/app/basics/chat)
  - *Config Presets & Hub:* [https://lmstudio.ai/docs/app/presets](https://lmstudio.ai/docs/app/presets)
  - *Prompt Templates & Jinja:* [https://lmstudio.ai/docs/advanced/prompt-template](https://lmstudio.ai/docs/advanced/prompt-template)
  - *Developer REST API:* [https://lmstudio.ai/docs/developer/rest/chat](https://lmstudio.ai/docs/developer/rest/chat)
  - *CLI `lms` Tooling:* [https://lmstudio.ai/docs/cli/local-models/chat](https://lmstudio.ai/docs/cli/local-models/chat)
