# Can the router use an existing Chorus agent account?

## Answer

Yes. Chorus can use one of its already-configured ACP-capable agent CLIs (for example, the configured Codex or Claude bridge) to make the pre-dispatch choice. That does not require Jev or a new API key: Chorus would spawn the selected bridge under the agent/account configuration it already uses and send it a small routing prompt. This is an additional real model inference, though, so it consumes that provider account's included usage or API billing. Jev's own pricing is not relevant unless Chorus actually calls Jev.

The router must be opt-in/configurable or have an explicit provider/model setting. It cannot be assumed free. If the configured CLI account uses subscription quota, the routing prompt draws from that quota; if it uses API credentials, it is billed under that API account. OpenAI states Codex signed in with ChatGPT uses the plan's shared Codex/Work allowance, while API-key use follows API pricing. Anthropic states Claude and Claude Code share plan usage limits; Console/API auth instead uses API billing. [OpenAI usage and billing](https://help.openai.com/en/articles/20001275-chatgpt-work-and-codex) · [Anthropic Claude Code plan usage](https://support.anthropic.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan)

## What Chorus already does

- `refresh_models` launches a short-lived ACP bridge, opens `session/new`, reads the bridge's `models.availableModels`, then caches the IDs and labels. It deliberately sends no prompt. Source: [`crates/chorus-cli/src/agents.rs`](../../../crates/chorus-cli/src/agents.rs#L95).
- For each actual dispatched turn, `one_turn_decide` resolves the selected agent's bridge and account environment, constructs `CodexConfig`, starts `CodexAgent`, opens a new ACP session, pins the configured model, and prompts it. The agent bridge is therefore not a persistent per-session worker in this path: current turns incur bridge process/session startup. Source: [`crates/chorus-cli/src/session.rs`](../../../crates/chorus-cli/src/session.rs#L1194).
- `CodexAgent::start` performs `spawn_and_init → session/new → pin_mode → pin_model`; `session/new` returns the session ID and the current/available models. Source: [`crates/chorus-acp/src/codex.rs`](../../../crates/chorus-acp/src/codex.rs#L205).
- The account environment is explicitly passed into that subprocess. `bridge_for` chooses the provider bridge by agent name, with command overrides available. Source: [`crates/chorus-acp/src/codex.rs`](../../../crates/chorus-acp/src/codex.rs#L20) and [`crates/chorus-cli/src/session.rs`](../../../crates/chorus-cli/src/session.rs#L1230).
- Chorus already passes a model pin and verifies the model selected by the bridge when one is reported. For Codex, model IDs reported by the bridge can include an effort suffix (e.g. `gpt-5.5[low]`); `pin_model` first tries `session/set_model`, then falls back to the `model` config option. Source: [`crates/chorus-acp/src/codex.rs`](../../../crates/chorus-acp/src/codex.rs#L285).

So a distinct model-based router would normally add one more bridge startup/session and one routing inference before launching the chosen agent's existing per-turn bridge/session. The protocol client cannot make a separate model inference just by enumerating models; model listing is metadata returned during session creation. A deterministic local router would avoid that inference cost, but would not be the requested model-based chooser.

## ACP and effort support

ACP session setup exposes agent-reported model choices, and the ACP adapter can accept user prompts in a session. The protocol describes sessions as independent conversation contexts and places model/session configuration under adapter capabilities. The current ACP specification has evolved: v1 exposes the legacy models surface used by Chorus, while the current v2 migration guide says to drive model/mode UI from `configOptions` and set them with `session/set_config_option`. [ACP v1 session setup](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/protocol/v1/session-setup.mdx) · [ACP v2 migration guide](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/protocol/v2/migration.mdx)

The current Codex ACP adapter explicitly supports ChatGPT, API-key and custom-gateway authentication, as well as model and reasoning-effort configuration. Its server publishes a reasoning-effort select config option and validates requested efforts against the chosen model's supported efforts. [Codex ACP README](https://github.com/agentclientprotocol/codex-acp/blob/main/README.md) · [Codex ACP model/effort implementation](https://github.com/agentclientprotocol/codex-acp/blob/main/src/CodexAcpServer.ts)

Claude Code also has a documented `CLAUDE_CODE_EFFORT_LEVEL` setting, with model-dependent accepted values. Its ACP bridge can expose model configuration, but its exact live `configOptions` are adapter/version-specific; Chorus should discover config options from the active session and use advertised IDs/options rather than hardcoding generic effort strings. [Claude Code environment variables](https://code.claude.com/docs/en/env-vars) · [Claude ACP model configuration](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/docs/model-configuration.md)

**Current Chorus limitation:** this adapter stores model IDs and implements `set_model`, but `ModelInfo` only carries `id`, `name` and `is_current`; it does not parse the modern `configOptions` category/options metadata into a provider-neutral “reasoning effort” capability. The Swift picker recognizes a finite list of suffix values. For robust cross-agent routing, enumerate each agent's actual model and effort choices (or the model/effort config options) and only present choices that agent reports. If unsupported, preserve that agent's default and say so.

OpenAI's official guidance confirms the tradeoff: supported effort values are model-dependent; lower effort is faster and uses fewer reasoning tokens, while higher effort can improve planning/debugging at higher latency and token cost. It recommends low for routing/classification and evaluating quality, latency and cost per successful task. [OpenAI effort guidance](https://developers.openai.com/api/docs/guides/deployment-checklist#reasoning-effort)

## Cost, latency and context implications

1. **One more model request per user send.** The router's output generation, any reasoning tokens and the routing prompt count under whichever provider/model it uses. The provider's normal usage rules apply; no documentation supports a cost-free inference just because it selects a cheaper downstream model. Whether usage is tracked as one prompt, tokens or credits depends on provider/account/billing mode. Avoid promising a fixed price or quota amount.
2. **Additional startup and inference latency.** Chorus already opens a new subprocess and ACP session for the dispatched turn. A separate preflight call adds another such startup/handshake plus router model latency, before target-agent startup. No provider docs guarantee a fixed time; measure in the app. Reusing one process/session could remove some process startup if a pool is added, but would require lifecycle/account isolation and generic provider support; it is not how current turn dispatch is structured.
3. **Data exposure is whatever the router prompt contains.** ACP sessions receive a `cwd` and can be given MCP servers; the protocol's session setup does not itself require sending project files. The router needs only enough text/context to classify intent and the list of eligible agent/model/effort options. Keep it out of the main project `cwd`, omit MCP servers, and don't include ledger history, diffs, file contents or attachments by default. If the routing decision should depend on those, make that an explicit context setting/consent. This recommendation follows the ACP request shape and least-context design; built-in CLI agents may still have their own filesystem behavior tied to the supplied `cwd`, so use an empty/scratch cwd for the selector where feasible.
4. **Parallel or recursive routes multiply use.** If each reroute/agent handoff triggers a fresh selector request, it spends additional usage; cache/reuse a decision only when the exact task inputs and available options remain unchanged.

## Separate process/session decision

For a provider-neutral implementation, assume the selector is its own ACP session, and in the current Chorus architecture, its own bridge subprocess. This keeps routing context short and prevents the selected worker's prompt/history from contaminating selection. A router can use the same already-authenticated local provider CLI/account; it need not be a Jev service. Do not include the target agent's tools unless routing itself genuinely needs them.

An optimization could later keep a router bridge process warm, or use a local rules-first fast path and call the model only for ambiguous/high-impact tasks. That changes resource management and needs separate product decisions. It is not required to prove feasibility.

## Recommendation for the Wayfinder decision

- Treat router choice as a user-configurable, metered preflight service backed by an existing configured agent account, with provider/model/effort selection and a local/manual fallback.
- Make each bridge report supported model and effort options; use only those. Current Chorus supports model selection, but generic effort metadata needs implementation work.
- Explain to the user that each automatic routing decision consumes quota/credits or API spend and adds a model round-trip. Offer a no-model/manual routing option.
- Keep the selector prompt payload minimal and avoid forwarding attachments/history unless that is explicitly designed and disclosed.

## Evidence boundaries

No authenticated account was used, no provider calls were made, and no secrets were read. ACP/provider support is documented and source-level feasible; the exact available model/effort choices and billing path depend on the locally configured bridge, account, CLI version and plan. This report does not assert the present installed bridge versions or an exact latency/price.
