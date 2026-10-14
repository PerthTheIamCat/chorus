# Chat app survey

Type: research
Status: resolved
Blocked by:

## Question

What do good chat apps offer that a Chorus chat mode should match? Cover the ChatGPT and Claude desktop apps, LibreChat, T3 Chat, Msty, Open WebUI, and LM Studio. Record: model picker UX (switching mid-conversation, per-message model), branching and editing messages, regenerate, compare answers, search across history, projects and folders, attachments, artifacts and canvas, system prompts and personas, local-model support, and how chat is promoted into an agentic or coding task. Note which of these fit a no-project, read-only Chorus session backed by agent CLIs and local Ollama.

## Answer

- **Mid-conversation model switching**: Because the SQLite ledger is the single source of truth rather than a provider session (`CLAUDE.md` §2.2), Chorus can switch between local Ollama and cloud agent CLIs (Codex, Claude, Gemini) on every turn without state loss.
- **Branching, editing, and regeneration**: Surveyed apps (LibreChat, ChatGPT, Msty) rely on `< 1/N >` pagination and tree forking; in Chorus, editing or regenerating maps directly to `ledger::fork` and `ledger::history` over content-addressed blobs (`blob.rs`).
- **Answer comparison (Dual / Arena)**: Msty's Split Chat (`Cmd+T`) and Open WebUI's Compare mode match Chorus's existing `chorus compare` engine; chat mode should allow concurrent dual-model turns displayed in a two-column split view.
- **View-first Artifacts over Canvas**: Claude and LibreChat's side-drawer Artifacts (rendering code, Markdown, SVG, Mermaid) strictly fit Chorus's view-first philosophy (`map.md`), avoiding heavy in-app editing or IDE features.
- **Local Ollama as a primary backend**: Parity with Msty and Open WebUI is achieved by treating local Ollama (`localhost:11434`) as an equal peer to agent CLIs, enabling offline, zero-key chat.
- **Search and sidebar organization**: Leverage built-in SQLite FTS5 (`schema.rs`) for instant `Cmd+K` history search; keep chat threads in a flat "Chats" section in the sidebar above workspace projects.
- **Promotion into agentic tasks**: The primary advantage over surveyed tools is a seamless "Promote to Task" handoff: selecting a `<cwd>`, distilling chat history into a `Brief` (`handoff.rs`), initializing shadow git (`checkpoint.rs`), and escalating from read-only chat to multi-agent workbench execution.

Full survey, comparison matrix, and architectural analysis: [04-chat-app-survey.md](../research/04-chat-app-survey.md).
