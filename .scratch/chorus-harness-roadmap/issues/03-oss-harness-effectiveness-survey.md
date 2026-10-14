# What makes agents effective in OSS harnesses

Type: research
Status: resolved
Blocked by:

## Question

Which harness features measurably improve agent effectiveness? Survey well-regarded open-source and CLI harnesses: OpenCode, Cline, Roo Code, Goose, Aider, Crush, OpenHands, Warp, Claude Code itself (hooks, skills, subagents, plan mode, todo, CLAUDE.md), and Codex CLI (AGENTS.md). Also read first-party harness-engineering write-ups from Anthropic, OpenAI, and Cognition. Record each feature together with the evidence for it: context engineering, repo maps, rules/instructions files, hooks, plan mode, todo tracking, LSP diagnostics fed back to the agent, test and lint feedback loops, tool design, permission modes, and output compaction. Separate what Chorus could provide as the harness (so every agent benefits) from what each agent already does internally.

## Answer

- **Scaffolding Drives Double-Digit Performance Swings**: Empirical studies show that harness design often outweighs model upgrades—yielding **+10.7 pp** from Agent-Computer Interface design ([SWE-agent, NeurIPS 2024](https://arxiv.org/abs/2405.15793)), **+12–13 pp** from test/compiler execution feedback loops ([Self-Debugging, ICML 2023](https://arxiv.org/abs/2304.05128)), and jumping file localization from **~30% to >70%** via AST RepoMaps ([Aider](https://aider.chat/2023/10/22/repomap.html)).
- **Error-Proof Tool Design & Output Clipping Prevent Context Collapse**: Anthropic's SWE-bench Verified 49% scaffold ([Anthropic, Jan 2025](https://www.anthropic.com/engineering/swe-bench-sonnet)) demonstrated that minimal tools (`str_replace_editor` + `bash`), unique-match string editing, mandatory absolute paths, and truncated command outputs (`<response clipped>`) are mandatory to keep agents stable over long-horizon tasks.
- **Automated Verification Loops (LSP & Tests) Outperform User Prompting**: Feeding language server diagnostics (OpenCode, Cline/Roo Code) and automated test runner failures (Aider, Devin) directly into the agent observation fixes ~75% of syntax and type errors autonomously within one turn without human intervention.
- **Harness-Level vs. Agent-Internal Boundary**: Agents own model reasoning, token generation, and raw tool arguments. Chorus as the harness owns the Single Source of Truth event ledger, cross-agent handoff context assembly, git worktree isolation, shadow git checkpoints (`apply_reverse`), universal instruction projection (`AGENTS.md` / `CLAUDE.md`), and warm-tier Ollama compaction.
- **Ranked Recommendations for Chorus**:
  1. *Automated post-edit verification loop* (fast tests/linter run on commit).
  2. *Tool output compaction & guardrails* (blobs on disk, head/tail clipped in context).
  3. *Dynamic AST RepoMap* (Tree-sitter + PageRank in SQLite).
  4. *Universal rules projection* (`AGENTS.md` / `CLAUDE.md` injected across all ACP bridges).
  5. *Harness-level LSP diagnostics injection*.
  6. *Plan mode & task state scratchpad* in ledger.
  7. *Deterministic lifecycle hooks* (`PreToolUse`, `PostToolUse`).
  8. *Interactive permission gating & per-hunk rollback with inline review comments*.

Full survey with empirical evidence and primary source citations: [../research/03-oss-harness-effectiveness-survey.md](../research/03-oss-harness-effectiveness-survey.md)
