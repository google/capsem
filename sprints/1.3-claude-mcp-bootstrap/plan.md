# Claude MCP Bootstrap Sprint

> Superseded as a standalone execution slice by
> `sprints/1.3-release-correction/`, especially S9 Agent Bootstrap Repair.
> Keep this file as narrow Claude evidence only.

## Goal

Fix Claude startup so the built-in Capsem MCP server is predeclared and approved by profile bootstrap files. Claude must not prompt “New MCP server found in this project: capsem” for a fresh VM built from the checked-in profiles.

## Root Cause

The profile root ships `/root/.mcp.json` with the `capsem` MCP server and ships Claude global settings, but it does not ship `/root/.claude/settings.local.json`. Live VM evidence shows Claude writes `settings.local.json` with `enabledMcpjsonServers: ["capsem"]` only after the user accepts the prompt.

## Done

- Checked-in `code` and `co-work` profile roots include non-secret Claude MCP approval state.
- `root.manifest.json` pins the approval file hashes and sizes.
- Profile check/materialization tests fail if a profile declares `capsem` in `.mcp.json` but does not package Claude approval.
- Focused tests pass.
