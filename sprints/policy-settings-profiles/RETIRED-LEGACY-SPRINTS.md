# Retired Legacy Sprint Notes

Status: active guardrail

## Purpose

This file retires older sprint directories as planning authority for the
Profile V2 effort. They remain archived under `sprints/retired/` as historical
context and implementation archaeology, but they are not allowed to define
current product scope, sequencing, public surfaces, or release requirements
unless this `policy-settings-profiles` board explicitly imports a requirement.

## Active Authority

The active Profile V2 sprint authority is:

- `sprints/policy-settings-profiles/MASTER.md`
- `sprints/policy-settings-profiles/tracker.md`
- The numbered `Sxx-*` sprint files in `sprints/policy-settings-profiles/`

When those files disagree with older sprint directories, the
`policy-settings-profiles` board wins.

## Retired As Planning Authority

These directories are historical only for Profile V2 planning:

- `sprints/retired/mcp-policy-v2/`
- `sprints/retired/mitm-mcp-unification/`
- `sprints/retired/mitm-redesign/`
- `sprints/retired/mcp-endpoint-coverage/`
- `sprints/retired/observability-stop-the-bleeding/`
- `sprints/retired/release-policy-hardening/`
- `sprints/retired/release-debug-loop/`
- `sprints/retired/analytics-dashboard/`
- `sprints/retired/better_stats/`
- `sprints/retired/next-gen/`
- `sprints/retired/profile-v2-generated-settings-quarantine/`
- `sprints/retired/profile-v2-http-cel-builder-hardening/`
- `sprints/retired/profile-v2-migration-rescue/`
- `sprints/retired/profile-v2-remove-legacy-policy-config/`
- `sprints/retired/profile-v2-runtime-hygiene/`
- `sprints/retired/profile-v2-s07-uds-api/`
- `sprints/retired/profile-v2-test-fix/`

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
