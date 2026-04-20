# Meta sprint: credential-pipeline

Generalizing host credential detection into a declarative,
inspectable, extensible system. This sprint lands the foundation;
each follow-up adds a specific connector or consumer that reuses it.

## Status

| Sprint | Status | What it delivers |
|---|---|---|
| **credential-pipeline** (this file + `plan.md` / `tracker.md`) | Not Started | Declarative detection, `capsem detect` CLI + `GET /detect`, Connectors scaffold in settings + UI |
| [followup-google-workspace.md](followup-google-workspace.md) | Not Started | `connectors.google_workspace` — gws CLI, token detection, VM install |
| [followup-gcloud.md](followup-gcloud.md) | Not Started | `connectors.gcloud` — gcloud SDK / ADC, VM install |
| [followup-github-oauth.md](followup-github-oauth.md) | Not Started | `connectors.github` — OAuth beyond the PAT already in `repository.providers.github.token` |
| [followup-inventory-injection.md](followup-inventory-injection.md) | Not Started | Injection UX: let users opt specific detected skills / MCP servers into their VMs |

## Phase grouping

**Phase A — foundation (this sprint)**
- Declarative spec + walker.
- `capsem detect` CLI + `GET /detect`.
- Connectors namespace scaffolded. Inventory surfaced but report-only.

**Phase B — connectors (each is its own sprint)**
- Google Workspace, gcloud, GitHub OAuth. Each is a new
  `connectors.<name>.*` settings subtree + spec tables + VM install
  manager entry. No Rust changes on the detection side.

**Phase C — inventory consumers**
- Injection UX for MCP servers + skills. Requires product decisions
  about per-item opt-in and storage. Keep out of Phase A.

## Just recipes relevant

```
just test            # gates for Phase A commits
just smoke           # /detect round-trip after commit 2
just ui              # manual check after commit 3
just build-assets    # needed for any connector sprint that adds a VM install
```

## Conventions

- Every file in this directory is checked into git.
- Trackers have checkbox state. Commits update the tracker in the
  same commit as the code change.
- Follow-up sprint files are one-page stubs today; they get filled
  out when each sprint kicks off.
- Nothing in here gets deleted -- trackers are history.
