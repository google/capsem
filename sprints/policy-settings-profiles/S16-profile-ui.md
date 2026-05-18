# S16 - Profile UI

## Goal

Make signed/catalog profiles, profile revisions, package contracts, asset
readiness, and profile-backed VM creation first-class in the UI.

## Tasks

- Add profile selector.
- Add catalog/profile list with the canonical `ProfileRevisionStatus` enum:
  `active`, `deprecated`, and `revoked`. Do not display `removed` as a status;
  absent revisions are simply not offered.
- Add profile revision view: installed revision, catalog current revision,
  update availability, binary compatibility, payload verification state.
- Add create, fork, delete flows for user-authored profiles, while clearly
  separating catalog-installed corp/base profiles from editable user profiles.
- Show icon, name, description, best-for, type, version/revision.
- Show package/tool contract and VM asset readiness for the selected profile.
- Add General, Appearance, AI Providers, MCP & Connectors, Skills, VM, Security.
- Make VM/session launch use an explicit selected profile id and resolved
  revision. The create flow must surface first-use asset download progress and
  block revoked/incompatible profiles.
- Show existing VM bindings: profile id/revision, package contract hash, pinned
  asset hashes, and drift/deprecation/revocation warnings.

## Coverage Ledger

- Unit/contract: profile UI model tests for all `ProfileRevisionStatus` enum
  values, revisions, package/tool contracts, asset readiness, and VM pin fields.
- Functional: create/fork/delete/select tests; update/install catalog revision;
  profile-backed VM create with asset readiness states.
- Adversarial: locked/forbidden profile actions, revoked profile, incompatible
  profile revision, stale catalog rollback warning, asset download failure,
  interrupted download retry, and legacy/unbound VM pin display.
- E2E/VM: launch session with selected profile revision and verified assets.
- Telemetry: UI links to status/debug provenance for profile revision and asset
  verification failures.
- Performance: profile switching remains responsive and does not trigger network
  fetches or hash scans on every selection change.
