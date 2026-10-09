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

`inspect-capsem-sandbox` depends on `inspect-ai`, `capsem` (`>=0.7.0`), `pydantic`, and `pyyaml`:

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

A string sandbox config is also accepted: a `compose.yaml` / `docker-compose.yml` path, a `Dockerfile` / `Containerfile` path, or a pre-built OCI image reference (`"python:3.12-slim"`).

## `vm` vs `container` modes

- **`execution_mode="vm"` (default)**: Commands and file operations execute directly inside an
  ephemeral Capsem micro-VM.
- **`execution_mode="container"`**: Commands execute inside a rootless OCI workload container provisioned by `capsem-init` (`Hypervisor.create(image="docker://<image>")`) and entered via `runc exec --cwd / -u 0 workload bash -c …` (selected automatically when `compose_file`, `image`, `dockerfile`, or `build` is set). The container image must provide `bash`, coreutils (`timeout`, `stat`, `base64`, `head`, `split`), `tar`/`gzip`, and `setpriv`/`su`/`getent`/`id` when executing as a non-root user. `connection()` (`capsem shell <vm_id>`) opens an interactive shell in the backing Capsem VM.

### Compose support

In `container` mode, `inspect-capsem-sandbox` parses single-service `compose.yaml` / `docker-compose.yml` files with `PyYAML`:

- **Supported service fields**: `image`, `build` / `dockerfile`, `working_dir`, `environment` (mapping or `KEY=VALUE` list, with project `.env` interpolation, Inspect `SAMPLE_METADATA_<KEY>` synthesis from scalar sample metadata, and operator-owned host environment allowlisting via `CAPSEM_INSPECT_ALLOWED_HOST_ENV`), `user`, `command`, `entrypoint`, `volumes` (host bind-mount sources inside the Compose directory or `CAPSEM_INSPECT_ALLOWED_HOST_PATHS` staged once at sample start as one-way, root-owned `0:0` copies capped at 256 MiB, preserving file modes), `healthcheck` (polled via `sh -c` with default `retries=3`, `interval` sleep capped at 2 s), `mem_limit` / `deploy.resources.limits.memory` (mapped to VM `ram_gb`), and `cpus` / `deploy.resources.limits.cpus` (mapped to VM `cpu_count`).
- **Host environment and path isolation**: Host `os.environ` is default-deny in Compose interpolation and bare `environment` keys unless allowlisted by the operator via `CAPSEM_INSPECT_ALLOWED_HOST_ENV` (explicit variable names or literal-prefix patterns; wildcard-only and character-class patterns are rejected), optionally narrowed by `CapsemSandboxConfig.allowed_host_env`. Host bind-mount `volumes` are restricted to the resolved `compose.yaml` directory (`os.path.realpath` containment) or `CAPSEM_INSPECT_ALLOWED_HOST_PATHS` (optionally narrowed by `CapsemSandboxConfig.allowed_host_paths`).
- **Unsupported**: Compose files without a non-empty `services:` mapping, multi-service `services:` (>1 service), `depends_on`, `env_file`, `ports`, non-default `network_mode` (including `none`, `host`, and `service:<name>`), `internal: true` networks, and privilege/namespace keys (`privileged`, `cap_add`, `cap_drop`, `devices`, `security_opt`, `sysctls`, `pid`, `ipc`, `uts`, `cgroup`) raise `ValueError`. Other unrecognized service keys log a `WARNING` and are ignored (`x-*` extension keys are ignored silently). `x-inspect_k8s_sandbox.allow_domains`, `template`, `allow_domains`, and `host_workspace_dir` raise `NotImplementedError`.

### Evaluator-granted host image builds (`Compose build:` & `Dockerfile`)

Host-assisted `Dockerfile` and Compose `build:` execution is **opt-in and disabled by default** (permission is never inferred from the presence of a `Dockerfile` or `build:` stanza). The evaluator process environment is the root authority (`CAPSEM_INSPECT_HOST_BUILD=1` and `CAPSEM_INSPECT_ALLOWED_HOST_PATHS`), and task config (`CapsemSandboxConfig(host_build=HostBuildGrant(...))`) can only narrow — never widen — those grants. When granted, `inspect_capsem` builds a Linux OCI image on the host (`--network none` and an isolated empty `DOCKER_CONFIG` by default), ingests the `docker image save` archive into a local content-addressed blob cache, and serves it over a loopback HTTPS OCI v2 registry (`127.0.0.1:5055` by default) with a per-VM `[images]` `ca_pem` trust anchor passed to `capsem-service`.

> **Evaluator environment authority (`CAPSEM_INSPECT_HOST_BUILD` & `HostBuildGrant`):** `Dockerfile` `RUN` instructions execute inside `docker build` on the **host Docker daemon** before the built image is pulled into the Capsem micro-VM by digest (`127.0.0.1:<port>/inspect-capsem/build@sha256:<digest>`). Without `CAPSEM_INSPECT_HOST_BUILD=1` in the evaluator environment, any `Dockerfile` or Compose `build:` config fails fast with `ValueError` (even if `host_build` is set on the task). A task passing `host_build=False` or `HostBuildGrant(enabled=False)` disables host builds for that task.
>
> **Explicit context & symlink containment (`CAPSEM_INSPECT_ALLOWED_HOST_PATHS` & `HostBuildGrant.allowed_contexts`):** Build context roots are **never** inferred from a `compose.yaml` file's parent directory. The evaluator must allowlist build context and `Dockerfile` directories via `CAPSEM_INSPECT_ALLOWED_HOST_PATHS` (optionally narrowed per task via `allowed_host_paths` or `HostBuildGrant(allowed_contexts=(...,))`). Non-`.dockerignore`d symlinks inside the build context whose `os.path.realpath` escapes the context root are rejected before `docker build` runs, and mounting `docker.sock` into a sandbox is forbidden.
>
> **Network, credentials, and cache integrity (`CAPSEM_INSPECT_BUILD_CACHE_DIR`):** Host builds default to `--network none` (operator ceiling `CAPSEM_INSPECT_BUILD_NETWORK=none|default`, optionally narrowed by `HostBuildGrant(network="none")`; `--network host` is always refused) and an isolated temporary `DOCKER_CONFIG` (`{"auths":{}}`) so ambient `~/.docker/config.json` credentials are never read unless `CAPSEM_INSPECT_BUILD_DOCKER_CONFIG` is explicitly set under an allowed host path. Cache keys hash non-`.dockerignore`d context files, `Dockerfile`, `args`, `target`, `network`, and target platform. On cache hits, manifest, config, and layer blobs are verified by SHA-256 and automatically rebuilt if missing or corrupted.

Host-side image builds require the `docker` CLI and `openssl` on `PATH`, plus an explicit image source admission rule in `~/.capsem/settings.toml` (matching the loopback registry `host:port`, `127.0.0.1:5055` by default) before starting `capsem-service`:

```toml
[images]
sources = ["127.0.0.1:5055"]
admit = ["127.0.0.1:5055/inspect-capsem/build"]
```

## Configuration reference (`CapsemSandboxConfig`)

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `execution_mode` | `"vm" \| "container"` | `"vm"` | Execute directly in the Capsem VM or inside an OCI workload container |
| `image` | `str \| None` | `None` | Pre-built OCI container image admitted in `~/.capsem/settings.toml` when `execution_mode="container"` |
| `cpu_count` | `int` | `4` | Virtual CPU count for the Capsem VM |
| `ram_gb` | `int` | `8` | Memory allocation in GiB for the Capsem VM |
| `working_dir` | `str \| None` | `None` (`"/workspace"` in `vm`; image `WORKDIR` or `"/"` in `container`) | Working directory inside the guest VM or container |
| `compose_file` | `str \| None` | `None` | Path to a single-service `compose.yaml` / `docker-compose.yml` file |
| `dockerfile` | `str \| None` | `None` | Path to a standalone `Dockerfile` / `Containerfile` (requires `CAPSEM_INSPECT_HOST_BUILD=1`) |
| `build` | `dict[str, Any] \| None` | `None` | Normalized Compose `build:` specification (requires `CAPSEM_INSPECT_HOST_BUILD=1`) |
| `host_build` | `HostBuildGrant \| bool \| None` | `None` | Task-level host-build policy narrowing `CAPSEM_INSPECT_HOST_BUILD` and `CAPSEM_INSPECT_ALLOWED_HOST_PATHS` |
| `environment` | `dict[str, str]` | `{}` | Default environment variables passed to the VM session |
| `command` | `tuple[str, ...] \| str \| None` | `None` | Override container command |
| `volumes` | `tuple[str, ...]` | `()` | Host bind-mount specifications (`host_path:container_path[:mode]`) |
| `healthcheck` | `dict[str, Any] \| None` | `None` | Container healthcheck readiness polling configuration |
| `mem_limit` | `str \| None` | `None` | Container memory limit (for example `"2g"`), mapped to VM `ram_gb` |
| `user` | `str \| None` | `None` | Default execution user (`uid`, `uid:gid`, or username) |
| `allowed_host_env` | `tuple[str, ...]` | `()` | Optional task-level subset narrowing `CAPSEM_INSPECT_ALLOWED_HOST_ENV` |
| `allowed_host_paths` | `tuple[str, ...]` | `()` | Optional task-level subset narrowing `CAPSEM_INSPECT_ALLOWED_HOST_PATHS` |

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
- `CAPSEM_INSPECT_ALLOWED_HOST_ENV`: Operator-owned comma-separated list of host environment variable names or literal-prefix patterns permitted in Compose interpolation and bare `environment` entries.
- `CAPSEM_INSPECT_ALLOWED_HOST_PATHS`: Operator-owned list of host directory or file roots permitted for bind-mount sources and `Dockerfile` / `build:` contexts.
- `CAPSEM_INSPECT_HOST_BUILD`: Set to `1` (`true`) in the evaluator environment to authorize host-side `docker build` execution, or `0` (`false`) to deny (defaults to disabled).
- `CAPSEM_INSPECT_BUILD_NETWORK`: Operator ceiling for `docker build --network` (`none` or `default`; defaults to `none`; `host` is refused).
- `CAPSEM_INSPECT_BUILD_DOCKER_CONFIG`: Optional operator-allowlisted `docker --config` directory when pulling private base images during host builds.
- `CAPSEM_INSPECT_BUILD_TIMEOUT`: Timeout in seconds for host `docker build` and `docker image save` commands (defaults to `600`).
- `CAPSEM_INSPECT_BUILD_REGISTRY_PORT`: Loopback HTTPS OCI v2 registry port (defaults to `5055`; must match `sources` and `admit` under `[images]` in `~/.capsem/settings.toml`).
- `CAPSEM_INSPECT_BUILD_CACHE_DIR`: Directory storing cached OCI blobs and local TLS certificates (defaults to `~/.cache/capsem/inspect-oci-builds`).
- `CAPSEM_INSPECT_BUILD_PLATFORM`: Target platform passed to `docker build --platform` (defaults to `linux/arm64` on `aarch64`/`arm64` hosts and `linux/amd64` on `x86_64` hosts).
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
