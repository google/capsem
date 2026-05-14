# Startup And Install Contract

Last updated: 2026-05-13

## Purpose

This file defines the product behavior we want before comparing against the
current code. Code should be changed to match this contract, not the other way
around.

## Core Gate

The release gate for startup/install reliability is:

```bash
capsem uninstall
just install
capsem status
```

S1 expands `capsem status` into this release health gate. Until then, the gate
is approximated with explicit checks for installed binaries, service
registration, service liveness, gateway status, asset status, setup status,
UI/tray/app launchability, and saved VM preservation.

## Terms

- Runtime state: binaries, app bundle, tray/gateway/service binaries, launch
  agent/systemd unit, service process, temp VM state, stale sockets, runtime
  wiring.
- Durable user state: user config, corp config, credential references, saved VM
  metadata, persistent VM data, and audit/session/log data that policy requires
  us to preserve.
- Asset cache: downloaded rootfs/kernel/initrd blobs for current or previous
  versions.
- Saved-VM-referenced assets: asset blobs required to reopen a saved VM. These
  are protected until the saved VM is deleted or migrated.

## Uninstall, Purge, Update

`capsem uninstall` removes the installed runtime. It stops service/UI/tray
processes, unregisters launch agents, removes installed binaries/app/runtime
wiring, clears stale sockets, and removes temp VM state. It preserves durable
user state and any asset blobs referenced by saved VMs.

`capsem purge` is the destructive reset. It removes the runtime and durable
state, including configs, saved VM metadata/data, logs subject to policy,
credential references, and assets. It requires explicit confirmation.

Implementation note: the current checkout already has a top-level
`capsem purge` command for session cleanup. S7 must reconcile that CLI naming
before shipping a whole-product destructive purge. The S1 code path now fixes
the urgent side of the contract: `capsem uninstall` is runtime removal, not
durable-state deletion.

Update is runtime replacement:

1. verify the new payload is installable.
2. run runtime uninstall.
3. install the new runtime.
4. start the service.
5. prove health with `capsem status`.

Update never runs purge.

## Ownership Boundaries

| Owner | Responsibilities |
| --- | --- |
| Installer/package/update | Verify payload, uninstall old runtime, install new runtime, start service, surface install diagnostics. |
| `capsem status` | Report whether the installed product is coherent across binaries, service, gateway, app/tray, setup, assets, and durable-state policy. |
| Service | Own asset supervision, saved VM inventory, runtime health, readiness status, and retry/error state. |
| Setup | Own config/onboarding: corp config, security preset, provider credentials, repo/GitHub detection, user choices. |
| UI/wizard/dashboard | Present startup truth and recovery actions without hiding blocked or updating states. |
| Tray/app/gateway/CLI | Consume and project the same service status model as the UI. |

## Status And Doctor

`capsem status` is the host/install/startup health gate. It reports whether the
installed runtime is coherent enough to use and exits non-zero when blocking
health issues remain.

`capsem doctor` is the deeper VM diagnostic. It must call the same reusable
status health check before provisioning a diagnostic VM. If status is blocked,
doctor fails early with the status blockers instead of hiding install/startup
problems behind VM diagnostics.

Status blockers are modeled as typed issue variants first. Each blocker carries
a stable machine code, severity, variant payload, and structured report before
rendering as a human message at CLI/UI boundaries. The status contract must not
depend on parsing message strings.

`capsem status --json` emits the `capsem.status.v1` report shape for install
harnesses, UI, and future gateway/tray consumers. The text view is presentation;
the JSON report and typed issue codes are the contract.

The current CLI status slice checks host helper binaries, service unit
installation/path freshness, signed asset manifests, setup-state honesty,
service version, gateway version/token reachability, and defunct sessions.
Service-owned asset supervisor states are still an S3 follow-up; the interim
asset checks verify the local signed manifest and required boot files directly.

## Service Asset Model

The service has an asset supervisor. It runs on service start, periodically, and
after installed-version metadata changes. It does not need a special installer
RPC to reconcile assets.

Asset status states:

- `checking`: service is computing required assets and verifying local blobs.
- `updating`: required assets are missing or stale and download/verification is
  in progress.
- `ready`: current runtime assets are present and verified. Saved-VM dependency
  gaps are reported in a separate field so new VM creation is not blocked by an
  old saved VM, while `capsem status` can still fail the install/update gate.
- `error`: asset supervision cannot proceed. The status includes whether retry
  is possible, last error, current artifact, missing artifact identities, and
  enough detail for UI and `capsem status`.

Asset progress should include artifact name, digest or version identity, bytes
downloaded when available, total bytes when available, and phase
(`fetching-manifest`, `downloading`, `verifying`, `installing`, `complete`,
`failed`).

## Saved VM Asset Dependencies

A saved VM depends on the base assets it was created with. Saved VM metadata
must record:

- architecture.
- rootfs digest and version identity.
- kernel digest and version identity.
- initrd digest and version identity.
- guest ABI or image compatibility identity.
- created Capsem version and last successful run Capsem version.

Cleanup policy:

- Temp VMs do not protect assets.
- Saved VMs protect referenced rootfs/kernel/initrd blobs.
- Current-version assets are required for new VM creation.
- Missing saved-VM assets are reported separately from current-version assets
  that are still updating.
- `capsem status --json` treats missing saved-VM dependencies as typed
  `saved_vm_asset_missing` blockers.

## Setup Contract

Setup should run after the service is live. Setup may continue while assets are
`checking` or `updating`, because provider detection, security configuration,
corp config, and repo discovery do not require VM assets.

Setup must not:

- own the asset download lifecycle.
- mark VM readiness when the service says assets are not ready.
- silently swallow service, settings, provider, or corp config failures.
- make the UI believe install is complete when startup truth is still unknown.

Setup should:

- be idempotent after reinstall/update.
- fan out independent detection work where safe.
- preserve accepted user choices.
- return structured status that the UI can continue from.

## UI, Wizard, Dashboard, Tray

User-facing surfaces must represent the same states:

- service unavailable.
- service starting.
- install check failed.
- assets checking.
- assets updating with progress.
- saved VM dependency missing.
- setup incomplete.
- setup/config error.
- ready.

Create/run actions are disabled until prerequisites are ready. Disabled states
must explain the blocking reason. Retry actions appear only when the service or
setup reports a retryable failure.

## Verification Philosophy

The meta-sprint is not complete because a unit test passes. It is complete only
when installed-product paths pass.

Required proof classes:

- Unit/contract tests for uninstall/purge policy, status models, saved VM asset
  references, setup readiness decisions, and install check report schema.
- Functional tests through CLI/service/gateway APIs.
- Adversarial tests for partial install, stale launch agents, corrupt assets,
  missing binaries, dead service, unreachable release source, malformed settings,
  and saved VM dependency loss.
- E2E install tests for clean install, reinstall, uninstall/install, update, and
  purge.
- UI tests for wizard/dashboard/tray/app status rendering.
- Diagnostic evidence capture for failed gates.

## Current Known Mismatch

An earlier narrow patch made setup fail when asset download failed. That patch
proved the old chain could falsely complete, but it conflicted with this
contract because asset supervision belongs to the service and setup should keep
config work moving while assets update in the background.

That patch has been reverted. S1/S3/S5 replace it with install diagnostics,
service-owned asset status, and setup readiness honesty.
