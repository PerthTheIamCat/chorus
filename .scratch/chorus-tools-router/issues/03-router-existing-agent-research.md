# Router feasibility with existing agent accounts

Type: research
Status: resolved

## Question

Can Chorus run a pre-dispatch routing decision through a model available from an already-configured agent account, without calling the Jev API or requiring a separate API key? Trace the current model enumeration, ACP session lifecycle, set-model/reasoning controls, and dispatch path. Use current primary docs to identify account/quota implications, extra startup/latency cost, model capability metadata, and whether a separate selector process/session is needed. Do not read secrets or make authenticated live calls.

## Answer

A separate pre-dispatch inference can run through an already-configured ACP agent account without Jev or a separate API key. It consumes that provider's quota or API budget and adds a bridge startup plus inference latency. Chorus can currently enumerate models and set model IDs, but reasoning-effort discovery is not provider-neutral; the router should use each active bridge's advertised model/config options. Keep selector context small and provide a manual/no-router fallback. See the [router feasibility report](../research/03-router-existing-agent.md).
