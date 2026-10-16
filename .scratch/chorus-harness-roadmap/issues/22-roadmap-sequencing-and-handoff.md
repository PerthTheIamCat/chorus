# Roadmap sequencing and handoff

Type: grilling
Status: resolved
Blocked by: 10, 11, 12, 14, 15, 16, 17, 18, 19, 20, 21

## Question

With every area decided, confirm the final phase order, the dependencies between areas, and the milestone exit criteria (tied to the harness eval). Then split the roadmap into per-area implementation specs ready for `.scratch/` implementation issues, and write the roadmap spec that this map hands off.

## Answer

- **Confirmed 6-Phase Sequence & Milestones**:
  - **Phase 1: Foundation (Core & Scaffolding)**:
    - Scope: Headless Daemon (`chorusd`), UDS socket & SSE event streaming with `Last-Event-ID` replay (Ticket 20), Agent Control Surface CLI & Streamable-HTTP MCP with AI Role Generator (Ticket 11), Harness Eval Suite (Ticket 10), Post-edit verification hooks, Universal Project Rules (`AGENTS.md`), and Granular permissions (Ticket 21).
    - Exit Criteria: `chorusd` runs standalone; Tier 0 (<2s) and Tier 1 (<30s) eval test suites pass with 100% reliability.
  - **Phase 2: Workbench UI (View-First Architecture)**:
    - Scope: 4-Column Adaptive Layout with Floating Glass Cards (Ticket 13), Virtualized AppKit `NSOutlineView` File Tree with Agent Focus Ring `●`, TextKit 2 Code Viewer, and `⇧⌘E` External Editor Handoff (Ticket 14), Unified & Split Diff Viewer with Myers diff in Rust, spacer line alignment, gutter hover comments, and `⌘↩` steering loop (Ticket 15), Plan Checklist Card and 1-Click Git Commit/PR flow (Ticket 21).
    - Exit Criteria: 120Hz scrolling on 50,000+ files; inline comments seamlessly feed into agent turns; external editor launches at exact line in <300ms.
  - **Phase 3: Multi-Agent & Orchestration**:
    - Scope: Subagent Visibility UX & Bridge Normalizer for Codex and Claude (Ticket 06, 16), Sub-200ms Git Worktrees with `.worktreeinclude` and dynamic `$PORT` allocation (Ticket 12), and Dynamic port browser launcher (Ticket 21).
    - Exit Criteria: 3 concurrent write agents execute in parallel with zero git locks or file conflicts; child prose/tools cleanly segregated from parent; parent receives compact structured brief.
  - **Phase 4: Router Quality & Analytics**:
    - Scope: Local In-Process Embedding Probe (3–8ms in Rust, no Ollama daemon) with cost-biased utility defaulting to low-effort/cheaper models and removal of 5s countdown (Ticket 07, 17), SQLite kNN override flywheel, and Monetary Cost Accounting (Ticket 21).
    - Exit Criteria: Routing latency <10ms; zero over-picking on simple tasks; >60% routine tasks routed to efficient models with zero quality regression on eval suite.
  - **Phase 5: Chat Mode & Attachments UX**:
    - Scope: Workspace-Free Chat Mode with mid-turn model switching and "Promote to Task" workflow (Ticket 18), Premium Attachment UX with clipboard `⌘V` thumbnail staging, Quick Look spacebar preview, `⌘L` context chip, and SF Symbols vision badges (Ticket 19), SQLite FTS5 cross-session search and session export (Ticket 21).
    - Exit Criteria: Zero-project instant chat starts; clipboard image paste and Quick Look under 1s; FTS5 search across 1,000 sessions under 50ms.
  - **Phase 6: Remote-Ready Core Boundary**:
    - Scope: Validate headless `chorusd` daemon API, mTLS/WireGuard tunnel pairing protocol, and async approval streaming for future mobile app handoff (Ticket 05, 20).
    - Exit Criteria: Headless API ready for remote mobile client implementation.
- **Handoff Master Specification**: Produced at [ROADMAP.md](../ROADMAP.md).
