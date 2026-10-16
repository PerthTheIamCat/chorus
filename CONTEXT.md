# Chorus Context

Terms used in Chorus product discussions so features describe the same concepts across providers.

## Language

**Agent**:
A provider-backed runtime that can receive a turn and work in a session, such as the Claude or Codex CLI. An agent may expose multiple models.
_Avoid_: Model, when referring to the provider runtime

**Model**:
The specific model selected within an agent, with the reasoning effort it supports.
_Avoid_: Agent, when referring to a model choice

**Integration**:
An optional capability Chorus makes available to agents. Integrations can take different forms, including MCP servers, command-line tools, and skills.
_Avoid_: Tool, when the integration type matters

**Attachment**:
An image or file copied into Chorus session storage and associated with a message. Being stored does not by itself mean the selected agent can read its contents.
_Avoid_: Workspace path, when referring to the attached content itself

**Enabled integration**:
An installed integration made available to agents working in one project. Installing it for the user does not enable it in every project.
_Avoid_: Installed integration, when referring to project access

**Router**:
A decision step that selects an agent, model, and reasoning effort for a user turn before the work is dispatched.
_Avoid_: Agent, which performs the dispatched work

**Route proposal**:
The router's recommended agent, model, and reasoning effort before the user has overridden it or Chorus has dispatched the turn.
_Avoid_: Dispatched target, which is the final choice actually used

**Subagent**:
A child agent that an agent spawns inside its own turn, through the provider's own mechanism (for example, Claude Code's Task tool or a Codex spawned agent). Chorus observes subagents but does not create them.
_Avoid_: Parallel agent, when the child was started through Chorus

**Orchestrating agent**:
An agent that asks Chorus, through Chorus's agent-facing control surface, to start and coordinate other agents. Chorus itself never plans or splits the work.
_Avoid_: Router, which only picks a target for a user turn

**Chat session**:
A session not bound to any project, in which the answering model has read-only access and no workspace to change.
_Avoid_: Session, when the project-bound kind is meant

**In-process model**:
A locally loaded model executed directly inside the Chorus engine process via embedded runtime (e.g. Metal GGUF) without an external daemon.
_Avoid_: Ollama model, when referring to embedded weights

**Model manager**:
The Chorus component that downloads, verifies, and stores local model weights in user application support.
_Avoid_: Package manager

**Harness eval**:
A repeatable task suite run through the real Chorus session engine to measure whether Chorus helps agents succeed, judged by outcome, tokens, time, and interventions rather than by model quality alone.
_Avoid_: Bench, which measures storage and context assembly

