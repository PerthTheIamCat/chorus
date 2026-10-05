# Integration catalog options

Type: research
Status: resolved

## Question

Classify graft, Serena, RTK, and Headroom by their current official installation method, runtime/protocol (MCP, CLI, skill, or other), macOS support, update/removal behavior, and how an agent session can use them. Inspect Chorus's existing MCP and skill handling to identify reusable paths. Use official project documentation or repositories, note licensing/source provenance where available, and do not install anything.

## Answer

Chorus can already pass MCP servers through ACP and project skills through its projection path. Graft, RTK, and Headroom each combine multiple integration forms; Serena fits as MCP. MCP is the lowest-friction first catalog type, while RTK's shell rewriting and Headroom's proxy mode need separate runtime support. Installers can mutate user/project agent configuration, so the catalog needs source/version/license metadata and Chorus-owned, reviewable wiring. See the [integration catalog report](../research/02-integration-catalog.md).
