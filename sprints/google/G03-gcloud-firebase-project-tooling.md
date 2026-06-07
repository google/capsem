# G03 - gcloud And Firebase Project Tooling

## Goal

Support gcloud and Firebase project tooling without leaking credentials or
making project state implicit.

## Scope

- gcloud CLI account/project/config discovery.
- ADC lookup and service-account behavior.
- Firebase CLI/project selection.
- Project id, account, scope, and service enablement diagnostics.
- Profile-owned materialization into sessions.

## Acceptance Criteria

- Project/account selection is visible in status/debug.
- Service-account and ADC paths are redacted and audited.
- Firebase/gcloud failures explain missing auth, missing project, disabled API,
  and policy denial distinctly.
