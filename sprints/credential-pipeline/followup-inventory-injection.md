# Follow-up sprint: inventory injection

**Prereq:** credential-pipeline sprint merged.

## What

Let users opt specific detected skills and MCP servers into their
VMs. The credential-pipeline sprint surfaces both as report-only
inventory under `connectors.mcp_servers` and `connectors.skills`.
This sprint makes the selection persistent and wires the injection
through the image builder + guest boot.

## Why this is a separate sprint

Product choices we do NOT want to force into the foundation sprint:

- Per-item opt-in UX. "Inject all of my Claude Code MCP servers into
  every VM" is almost never what the user wants -- some servers carry
  tokens or contact unrelated infrastructure.
- Scope per VM. A named VM might want a different set than the
  ephemeral default.
- Refresh semantics. When the host adds a new MCP server, do we
  auto-inject on next boot or require re-confirmation?
- Secret handling. MCP server configs often include env vars with
  tokens. These currently detect into the report; injection means
  actually storing them server-side.

## Scope (first pass)

New settings under `connectors.mcp_servers.*` and
`connectors.skills.*`:

```
connectors.mcp_servers.inject       list of opt-in server names (text)
connectors.mcp_servers.per_vm       bool (default false) -- per-VM override vs global
connectors.skills.inject            list of opt-in skill names (text)
```

Image builder:
- Consume the opt-in list at build time, inject the selected MCP
  server configs into the guest's AI agent config files.
- Skills: copy selected skill directories into guest `~/.claude/skills/`
  or equivalent.

UI:
- Settings > Connectors > MCP Servers and Skills gain a toggle per
  item (opt into injection).
- After toggle, a "will apply on next VM boot" hint.

## Unresolved

- Whether skills inject as symlinks, copies, or mounted read-only.
- Secret redaction in UI when displaying MCP server config snippets.
- Host-side change detection: rebuild needed? boot-time read?

## Exit criteria

- Toggle on `capsem` MCP server in Settings, boot a VM, confirm it
  appears in the VM's Claude Code config.
- Skills toggle works for at least Claude Code; Gemini CLI path
  scoped or deferred depending on how skills discovery works there.
