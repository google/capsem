---
title: Debug Report
description: Collect the redacted JSON report used for Capsem bug reports and release triage.
sidebar:
  order: 2
---

The debug report is the first artifact to ask for when a user reports that an installed Capsem release is broken. It is redacted JSON, small enough to paste into a GitHub issue, and focused on release attribution rather than a full support archive.

## Collecting

From a terminal:

```bash
capsem debug
```

From the desktop app, open Settings -> About and click **Copy debug report**.

Both surfaces call the same service endpoint:

```text
GET /debug/report
```

## What It Contains

The JSON schema is `capsem.debug.v1`.

| Section | Purpose |
|---------|---------|
| `version` | Installed binary version, build hash, build timestamp, and platform. |
| `paths` | Redacted Capsem home, run, and assets directories. |
| `runtime` | VM counts plus service/gateway pid, port, and token-file presence. Token contents are never included. |
| `host` | Host OS, architecture, and OS family. |
| `disk` | Total and available bytes for Capsem home, run, and assets paths. |
| `install` | Installed bin directory, current executable path, and service unit path. |
| `host_binaries` | Path, size, mode, executable bit, and BLAKE3 hash for Capsem host binaries. |
| `processes` | Known service/gateway/tray/MCP pids, whether the pid is alive, and executable path/hash when known. |
| `status` | Readiness issues that explain why `capsem status` or the `capsem doctor` preflight would fail, plus defunct session summaries. |
| `setup` | `setup-state.json` presence and parsed install/onboarding flags. |
| `assets` | Manifest path/hash/signature metadata, resolved asset version, and kernel/initrd/rootfs manifest hashes, actual hashes, sizes, and match status. |
| `logs` | Redacted tails from service, gateway, tray, MCP, and latest doctor logs when present. |

## Reading Asset Failures

For release regressions, start here:

```json
{
  "version": {
    "capsem_version": "1.1.1778542197",
    "build_hash": "1d95b80.1778545863"
  },
  "assets": {
    "asset_version_for_binary": "2026.0512.1",
    "files": {
      "initrd": {
        "manifest_hash": "...",
        "actual_hash": "...",
        "actual_hash_matches_manifest": true
      }
    }
  }
}
```

If `actual_hash_matches_manifest` is false, the installed asset on disk does not match the manifest used by that binary. If `exists` is false for `kernel`, `initrd`, or `rootfs`, the install or asset update path failed before the VM could boot correctly.

Use `asset_version_for_binary`, the three asset hashes, and `version.build_hash` to map the user report back to the exact release payload.

## Reading Status Failures

Check `status.issues` before drilling into logs. It is the concise readiness list:

```json
{
  "status": {
    "issues": [
      "Initrd asset is MISSING: ~/.capsem/assets/initrd.img"
    ],
    "defunct_sessions": [
      {
        "name": "demo",
        "last_error": "boot failed before ready"
      }
    ]
  }
}
```

If `status.issues` is non-empty, it should explain why `capsem doctor` would refuse to run or why `capsem status` reports the install as unhealthy.

## Reading Setup Failures

Use `setup.install_completed`, `setup.completed_steps`, and `setup.vm_verified` to distinguish these cases:

| Symptom | Likely meaning |
|---------|----------------|
| `setup.present` is false | Setup never wrote `setup-state.json` or the install cleaned it unexpectedly. |
| `install_completed` is false | CLI setup did not finish mandatory install steps. |
| `vm_verified` is false | Setup did not prove a VM can boot/run after assets were installed. |
| `providers_done` is false | AI provider credential detection/import did not complete. |

## Privacy

The report redacts home-directory usernames and token-like log values such as bearer tokens, `token=...`, and `api_key=...`. It includes only short log tails. For deeper debugging, ask for `capsem support-bundle`; for in-VM network or sandbox proof, ask for `capsem doctor --bundle`.
