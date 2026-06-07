# F01 - Installed Profile Product Proof

## Goal

Prove the shipped Profile V2 product works from package install through first
VM/session use.

## Scope

- Package/UI waits for setup, service, and gateway readiness.
- Onboarding and Settings Profiles use installed profiles, not catalog
  emptiness.
- Dashboard starts sessions from visible profile cards.
- CLI `capsem status`, `capsem run "echo test"`, and `capsem shell` work after
  install.
- Repeated or interrupted installs keep profile metadata and assets coherent.
- Profile cards do not advertise unprovisionable profiles.

## Acceptance Criteria

- Installed UI proof is recorded with version and profile ids.
- Installed CLI/VM proof is recorded with command output summary.
- `release-hit-list.md` items are closed or mapped to later F-sprints.
