# Sprint: 1.3 VM Lifecycle Unification

## Why

The 1.3 profile model no longer presents users with "temporary" versus
"normal" VMs. A VM belongs to a profile, appears in one VM list, and exposes the
same lifecycle verbs wherever it is shown: pause/resume, stop/start, fork, and
delete. The UI must not offer a "save/persist" escape hatch or split sessions
by backend storage terminology.

## Scope

- Unify the Sessions VM table into one list.
- Make profile launcher and custom session creation create named retained VMs.
- Make row actions status-driven, not `persistent`-driven:
  - Running: Pause, Stop, Fork, Delete.
  - Stopped/Suspended/Error: Start/Resume, Fork, Delete.
- Apply the same action contract to the active VM toolbar menu.
- Burn user-visible temporary/ephemeral/persistent language from the frontend.
- Keep backend storage fields untouched in this slice unless compile fallout
  requires it; deeper CLI/backend vocabulary burn remains a separate runtime
  compatibility removal.

## Files

- `frontend/src/lib/components/shell/NewTabPage.svelte`
- `frontend/src/lib/components/shell/CreateSandboxDialog.svelte`
- `frontend/src/lib/components/shell/Toolbar.svelte`
- `frontend/src/lib/components/shell/App.svelte`
- `frontend/src/lib/stores/vms.svelte.ts`
- `CHANGELOG.md`

## Done

- No frontend user-facing `ephemeral`, `temporary session`, or `persistent`
  split remains.
- VM list exposes pause/resume, stop/start, fork, delete on each VM.
- Creation paths send `persistent: true` with a VM name.
- Focused VM toolbar exposes the same lifecycle verbs.
- Frontend check/build pass.

## Proof Matrix

- Functional: frontend creation/action wiring compiles against the route client.
- Adversarial: grep guard catches old user-visible VM-class wording in
  frontend components.
- E2E/UI: frontend build succeeds; browser/manual smoke remains for the larger
  final release gate.
- Missing: backend/CLI still expose internal `persistent` API fields and old
  commands; they are outside this UI cleanup and are tracked in the broader
  finalizing sprint.
