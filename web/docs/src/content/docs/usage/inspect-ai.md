---
title: Inspect AI Sandbox
description: Run Inspect AI evaluations inside Capsem micro-VMs with inspect-capsem-sandbox.
sidebar:
  order: 4
---

`inspect-capsem-sandbox` registers a `capsem` [`SandboxEnvironment`](https://inspect.aisi.org.uk/sandboxing.html)
for [Inspect AI](https://inspect.aisi.org.uk/) backed by isolated Capsem micro-VMs. It uses the
Python `capsem` SDK (`capsem.Hypervisor` and `capsem.VM`) to talk to the authenticated Capsem HTTP
gateway (`capsem-service`).

| What | Name |
| --- | --- |
| Distribution (`pip install`) | `inspect-capsem-sandbox` |
| Python import package | `inspect_capsem` |
| Inspect sandbox type | `capsem` |
| Repository directory | `integrations/inspect-ai` |

## Install and configure

`inspect-capsem-sandbox` depends on `capsem` (`>=0.7.0`), `inspect-ai`, and `pydantic`:

```sh
pip install inspect-capsem-sandbox
```

When installing from a repository checkout before `0.7.0` is published, install `./sdk/python` and
`./integrations/inspect-ai` from the same revision.

Run any Inspect evaluation in a Capsem VM sandbox from the CLI:

```sh
inspect eval task.py --sandbox capsem
```

Or declare the sandbox in a `@task` definition:

```python
from inspect_ai import Task, task
from inspect_capsem import CapsemSandboxConfig


@task
def my_eval() -> Task:
    return Task(
        dataset=...,
        solver=...,
        scorer=...,
        sandbox=(
            "capsem",
            CapsemSandboxConfig(cpu_count=4, ram_gb=8),
        ),
    )
```

## VM execution

Commands and file operations execute directly inside an
ephemeral Capsem micro-VM.

## Configuration reference (`CapsemSandboxConfig`)

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `cpu_count` | `int` | `4` | Virtual CPU count for the Capsem VM |
| `ram_gb` | `int` | `8` | Memory allocation in GiB for the Capsem VM |
| `working_dir` | `str` | `"/workspace"` | Default working directory inside the guest VM |
| `environment` | `dict[str, str]` | `{}` | Default environment variables passed to the VM session |
| `user` | `str \| None` | `None` | Default execution user (`uid`, `uid:gid`, or username) |

## Environment variables

Gateway discovery follows the same precedence as the `capsem` CLI:

- `CAPSEM_GATEWAY_URL`: Explicit gateway base URL (falls back to `<run_dir>/gateway.port`, else
  `http://127.0.0.1:19222`).
- `CAPSEM_GATEWAY_TOKEN`: Explicit bearer token (falls back to `<run_dir>/gateway.token`).
- `CAPSEM_RUN_DIR` / `CAPSEM_HOME`: `<run_dir>` is resolved from `CAPSEM_RUN_DIR`, else
  `$CAPSEM_HOME/run`, else `~/.capsem/run`.
- `CAPSEM_VM_PREFIX`: Optional per-run label slug for isolating concurrent `inspect eval` runs on a
  shared host. Sample VMs are created unnamed and ephemeral (`persistent=False`) with
  `labels={"managed-by": "inspect-capsem"}`; setting `CAPSEM_VM_PREFIX=<value>` normalizes `<value>`
  and attaches `inspect-capsem-prefix: <slug>` so leftover sweeps and
  `inspect sandbox cleanup capsem` only reap VMs matching the active prefix.
- `INSPECT_CAPSEM_SANDBOX_TOOLS_PATH`: Optional host path override for the `inspect-sandbox-tools`
  binary baked into `/var/tmp/sandbox-services/inspect-sandbox-tools`.

## Lifecycle, cleanup, and limits

- **Timeouts**: When `exec(..., timeout=None)` is called without an explicit timeout, the controller
  call is capped at Capsem's 3600 s service ceiling (`EXEC_TIMEOUT_CEILING_SECS`) without wrapping
  the guest command in `timeout -k 1s`. A service-side kill at the 3600 s ceiling returns a non-zero
  `ExecResult`, whereas a gateway/transport timeout raises
  `TimeoutError("Command timed out after 3600s")`.
- **Output and file-read limits**: When `exec` output exceeds
  `SandboxEnvironmentLimits.MAX_EXEC_OUTPUT_SIZE` (10 MiB per stream, or when the gateway sets
  `truncated=True` at its 10 MiB combined `stdout + stderr` capture cap), `exec` raises
  `OutputLimitExceededError` with `truncated_output` set to the trailing UTF-8 slice of `stderr`
  when `stderr` exceeded the limit and `stdout` otherwise. `read_file` checks file size via `stat`
  before downloading and raises `OutputLimitExceededError(..., truncated_output=None)` without
  reading the payload.
- **File permissions (`write_file`)**: `write_file(file, contents)` creates or overwrites `file`
  with default file permissions (`0644`). Its pre-write permission check runs as `root` on file mode
  bits only (`stat -c '%a'` masked with `0222`). To execute a staged script directly
  (`./script.sh`), run `chmod +x` via `exec` or invoke it through its interpreter
  (`bash script.sh`, `python3 script.py`).
- **Non-root `exec` environment**: When `exec` runs as a non-root user (via `exec(..., user=...)` or
  `CapsemSandboxConfig(user=...)`), it runs `su -m` (or `setpriv --reuid=… --regid=…
  --clear-groups` when `user` is a numeric `uid:gid` or a numeric `uid` without a `/etc/passwd`
  entry) and resets `USER`, `LOGNAME`, and `HOME` from the target user's `/etc/passwd` entry
  (`getent passwd`, falling back to the numeric UID), unless passed explicitly in
  `exec(..., env=...)`.
- **Signal handling and cleanup**: `inspect_ai` handles `SIGINT` cleanly (`sample_cleanup`,
  `task_cleanup`, and `@atexit` hooks). Under `systemd-run` or another supervisor, configure
  `KillSignal=SIGINT` (and `KillMode=mixed` under `systemd` so `uv run` does not deliver duplicate
  `SIGINT` signals). If a run is cancelled mid-create and the 25 s teardown wait expires, run
  `inspect sandbox cleanup capsem` to delete any remaining VMs carrying the
  `managed-by=inspect-capsem` label (and matching `inspect-capsem-prefix` when `CAPSEM_VM_PREFIX` is
  set).

