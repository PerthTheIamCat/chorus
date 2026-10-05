# Integration catalog trust and permissions

Type: grilling
Status: resolved
Blocked by: 02

## Question

Given the integration research, settle the catalog's first-release trust and lifecycle rules: which curated sources are allowed, how install/update/remove works at user scope, how project-level enablement is represented, what permissions each integration receives, and how Chorus handles unavailable or incompatible tools. The agreed direction is a curated catalog installed once for the user and enabled per project.

## Answer

- Ship curated, version-pinned entries for Graft, Serena, RTK, and Headroom, with source, license, digest, runtime requirements, install actions, and declared file/network/secret-name access. The catalog covers MCP, projected skills, and callable CLI in the first release. Hook injection and model-request proxy modes are separate future capabilities; a CLI entry does not silently activate them.
- Install into a Chorus-managed user directory; never run opaque vendor `init` commands or edit global agent configuration as part of a catalog install. Show the planned effects before initial installation. Keep the user's existing `.chorus/mcp.json` and skill files intact.
- Enable integrations per project. At agent session creation, merge enabled MCP entries with existing project config, project enabled skills through the established skill path, and expose enabled CLI executables only to that project's agent process. Report name collisions and incompatible/missing runtimes instead of overriding or silently failing.
- The user chose automatic updates. The curated manifest ships with Chorus releases, so no independent catalog update service is needed in the first release. Apply only versions approved there: verify the download, stage it, health-check it, then activate for new sessions. Preserve the prior version for rollback and do not change an active agent process mid-turn. Show update status/history and stop exposing a failed integration. An update requesting broader access or changed license terms waits for user review.
- Disabling a project does not uninstall the user-level tool. Uninstall removes only Chorus-owned files and must account for other enabled projects. Project exposure is a visibility control, not a claim of operating-system sandboxing.
- See the [implementation spec](../implementation-spec.md#2-curated-integration-catalog) and [source comparison](../research/02-integration-catalog.md).
