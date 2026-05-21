# Retired Legacy Sprint Notes

Status: active guardrail

## Purpose

This file retires older sprint directories as planning authority for the
Profile V2 effort. They remain in the repository as historical context and
implementation archaeology, but they are not allowed to define current product
scope, sequencing, public surfaces, or release requirements unless this
`policy-settings-profiles` board explicitly imports a requirement.

## Active Authority

The active Profile V2 sprint authority is:

- `sprints/policy-settings-profiles/MASTER.md`
- `sprints/policy-settings-profiles/tracker.md`
- The numbered `Sxx-*` sprint files in `sprints/policy-settings-profiles/`

When those files disagree with older sprint directories, the
`policy-settings-profiles` board wins.

## Retired As Planning Authority

These directories are historical only for Profile V2 planning:

- `sprints/mcp-policy-v2/`
- `sprints/mitm-mcp-unification/`
- `sprints/mitm-redesign/`
- `sprints/mcp-endpoint-coverage/`
- `sprints/observability-stop-the-bleeding/`
- `sprints/release-policy-hardening/`
- `sprints/release-debug-loop/`
- `sprints/analytics-dashboard/`
- `sprints/better_stats/`
- `sprints/next-gen/`

They may contain useful tests, bug history, or old implementation notes, but
they do not create live work unless a current sprint file names the carried
requirement.

## Rule For Future Work

Do not infer active Profile V2 scope from a retired directory. If a useful
capability is found there, promote it into a current sprint PRD with:

- the user problem,
- the product outcome,
- the profile/VM ownership model,
- dependencies,
- acceptance criteria,
- explicit non-goals.

Do not revive old endpoint shapes, command names, or subsystem ownership from
retired notes by accident.
