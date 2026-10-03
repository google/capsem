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

`inspect-capsem-sandbox` depends on `capsem` (`>=0.6.3`), `inspect-ai`, `pydantic`, and `pyyaml`.
It requires Python SDK APIs newer than the released `capsem 0.6.3`: those on `main`
(`EXEC_TIMEOUT_CEILING_SECS`, `GATEWAY_REQUEST_BUDGET_SECS`, `decode_exec_output`, and OCI
container fields on `Hypervisor.create`) plus `#286` (streaming large binary bodies), `#287`
(`CREATE_READY_SECS` and the `Hypervisor.create` `request_timeout`), and `Hypervisor.vm(id=...)`
foreign VM attachment. Until the next `capsem` release is cut, install both packages from the same
repository revision `<rev>`:

```sh
pip install \
  "capsem @ git+https://github.com/google/capsem.git@<rev>#subdirectory=sdk/python" \
  "inspect-capsem-sandbox @ git+https://github.com/google/capsem.git@<rev>#subdirectory=integrations/inspect-ai"
```

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
            CapsemSandboxConfig(execution_mode="vm", template="code", cpu_count=4, ram_gb=8),
        ),
    )
```

A string sandbox config is also accepted: a `Dockerfile` or `Containerfile` path, a
`compose.yaml` / `docker-compose.yml` path, or an OCI image reference (`"python:3.12-slim"`).

## `vm` vs `container` modes

`CapsemSandboxEnvironment` supports two execution modes:

- **`execution_mode="vm"` (default)**: Commands and file operations execute directly inside an
  ephemeral Capsem micro-VM created from the configured `template` profile (default `"code"`).
- **`execution_mode="container"`**: Commands execute inside a container running in the Capsem VM.
  This mode is selected automatically when the config names a `Dockerfile`, a `compose.yaml` /
  `docker-compose.yml`, or a container image:
  - **OCI workload container fast path**: When a single service is configured with a pre-built
    `image`, the default `template`, and none of `dockerfile`, `volumes`, `ports`, `expose`,
    `mem_limit`, `network_mode`, `user`, `init`, `healthcheck`, or `entrypoint`, the VM is created
    with the image directly (`Hypervisor.create(image="docker://<image>")`) and commands run in that
    workload container via `runc exec`.
  - **Nested Docker container (everything else)**: Otherwise `inspect-capsem-sandbox` starts a plain
    VM and runs the container with the guest Docker daemon (`docker run`). For a `Dockerfile` or a
    `compose.yaml` `build` section, the full build context directory is staged into the VM and built
    with `docker build` before the container starts.

In `container` mode every command, file read, and file write runs through `docker exec … bash -c`
(or `runc exec … bash -c` on the fast path), so **the container image must provide `bash`**.

### Compose support

In `container` mode, `inspect-capsem-sandbox` parses single-service `compose.yaml` /
`docker-compose.yml` files with `PyYAML`. The guest `dockerd` runs with `--bridge=none`, so
containers use `--network host` by default.

- **Supported service fields**: `image`, `build` (`context`, `dockerfile`, `args`, `target`, plus
  `Dockerfile` `WORKDIR` / `USER` extraction), `working_dir`, `environment` (mapping or `KEY=VALUE`
  list, with `.env` / OS environment interpolation), `user`, `command`, `entrypoint`, `volumes`
  (host bind-mount sources are copied into the VM once at sample start), `ports`, `expose`, `init`,
  `mem_limit` / `deploy.resources.limits.memory`, `network_mode` (`bridge` and `default` map to
  `host`; `service:<name>` is rejected), `healthcheck`, and `networks` (services attached only to
  `internal: true` networks run with `--network none`).
- **Unsupported**:
  - Compose files without a non-empty `services:` mapping, multi-service `services:` (>1 service),
    `depends_on`, and `env_file` raise `ValueError` (use `environment` or a project `.env` file
    instead of `env_file`).
  - Other unrecognized service keys (`shm_size`, `tmpfs`, `cap_add`, `privileged`, `cpus`,
    `platform`, `extra_hosts`, `dns`, `sysctls`, `ulimits`, etc.) log a `WARNING` and are ignored;
    `x-*` extension keys are ignored silently.
  - `x-inspect_k8s_sandbox.allow_domains`, `allow_domains`, and `host_workspace_dir` raise
    `NotImplementedError` because Capsem enforces network domain policy at the VM profile level and
    does not expose a host VirtioFS mount; configure domain policy in the Capsem profile and stage
    files with `write_file`.

## Configuration reference (`CapsemSandboxConfig`)

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `execution_mode` | `"vm" \| "container"` | `"vm"` | Execute directly in the Capsem VM or inside a container |
| `template` | `str` | `"code"` | Capsem profile name (`"default"`, `"code"`, or a custom profile configured on the gateway; unknown names raise `NotImplementedError`) |
| `image` | `str` | `"python:3.11-slim"` | Container image when `execution_mode="container"` |
| `cpu_count` | `int` | `4` | Virtual CPU count for the Capsem VM |
| `ram_gb` | `int` | `8` | Memory allocation in GiB for the Capsem VM |
| `working_dir` | `str` | `"/workspace"` | Default working directory inside the guest or container |
| `dockerfile` | `str \| None` | `None` | Path to a `Dockerfile` to build inside the VM |
| `build_context` | `str \| None` | `None` | Optional build context directory (defaults to `Dockerfile` parent) |
| `build_args` | `dict[str, str]` | `{}` | Build arguments (`--build-arg KEY=VAL`) passed to `docker build` |
| `build_target` | `str \| None` | `None` | Target build stage (`--target`) passed to `docker build` |
| `compose_file` | `str \| None` | `None` | Path to a `compose.yaml` / `docker-compose.yml` file |
| `environment` | `dict[str, str]` | `{}` | Default environment variables passed to container execution |
| `command` | `tuple[str, ...] \| str \| None` | `None` | Override container command |
| `entrypoint` | `tuple[str, ...] \| str \| None` | `None` | Override container entrypoint |
| `volumes` | `tuple[str, ...]` | `()` | Volume / bind-mount specifications (`host_path:container_path[:mode]`) |
| `ports` | `tuple[str, ...]` | `()` | Published container port mappings (`-p`) |
| `expose` | `tuple[str, ...]` | `()` | Exposed container ports (`--expose`) |
| `healthcheck` | `dict[str, Any] \| None` | `None` | Container healthcheck readiness polling configuration |
| `init` | `bool` | `False` | Run an init process inside the container (`--init`) |
| `mem_limit` | `str \| None` | `None` | Container memory limit (`--memory`, e.g. `"2g"`) |
| `network_mode` | `str \| None` | `None` | Container network mode (`--network`); defaults to `host` |
| `user` | `str \| None` | `None` | Default container user (`uid`, `uid:gid`, or username) |
| `build_timeout` | `int` | `3600` | Timeout in seconds for `docker build` (clamped to 1..3600 s) |

## Environment variables

Gateway discovery follows the same precedence as the `capsem` CLI:

- `CAPSEM_GATEWAY_URL`: Explicit gateway base URL (falls back to `<run_dir>/gateway.port`, else
  `http://127.0.0.1:19222`).
- `CAPSEM_GATEWAY_TOKEN`: Explicit bearer token (falls back to `<run_dir>/gateway.token`).
- `CAPSEM_RUN_DIR` / `CAPSEM_HOME`: `<run_dir>` is resolved from `CAPSEM_RUN_DIR`, else
  `$CAPSEM_HOME/run`, else `~/.capsem/run`.

Host-side image caching and staging limits:

- `INSPECT_CAPSEM_IMAGE_CACHE`: Set to `0` / `false` / `no` / `off` to disable host caching of built
  `Dockerfile` images (enabled by default).
- `INSPECT_CAPSEM_IMAGE_CACHE_DIR`: Host directory holding `<digest>.tar.gz` image archives
  (defaults to `~/.cache/inspect-capsem/images`).
- `INSPECT_CAPSEM_MAX_CACHE_IMAGE_BYTES`: Maximum uncompressed image size cached on the host
  (default 250 MiB).
- `INSPECT_CAPSEM_MAX_CACHE_TOTAL_BYTES`: Total cache directory cap before oldest archives are
  evicted (default 2 GiB).
- `INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES`: Maximum uncompressed size for `Dockerfile` build
  contexts (after `.dockerignore` filtering) and staged host bind-mount directories (default
  256 MiB; `0` disables the cap).
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
  bits only (`stat -c '%a'` masked with `0222`), which is more lenient than Docker when overwriting
  a root-owned `0644` file inside a container whose default `USER` is non-root. To execute a staged
  script directly (`./script.sh`), run `chmod +x` via `exec` or invoke it through its interpreter
  (`bash script.sh`, `python3 script.py`).
- **Non-root `exec` environment**: When `exec` runs as a non-root user (via `exec(..., user=...)` or
  a container's default non-root `USER`), it runs `su -m` to preserve container `ENV` variables and
  resets `USER`, `LOGNAME`, and `HOME` from the target user's `/etc/passwd` entry (`getent passwd`),
  overriding any image-level `ENV HOME` or `ENV USER` unless passed explicitly in
  `exec(..., env=...)`.
- **Signal handling and cleanup**: `inspect_ai` handles `SIGINT` cleanly (`sample_cleanup`,
  `task_cleanup`, and `@atexit` hooks). Under `systemd-run` or another supervisor, configure
  `KillSignal=SIGINT` (and `KillMode=mixed` under `systemd` so `uv run` does not deliver duplicate
  `SIGINT` signals). If a run is cancelled mid-create and the 25 s teardown wait expires before a
  cold pull finishes, run `inspect sandbox cleanup capsem` to stop any remaining
  `inspect-capsem-*` VMs.

## How it works

### In-guest `Dockerfile` builds and host image cache

Each sample runs in its own isolated micro-VM, so images built with `docker build` inside a sample
VM's ephemeral `dockerd` disappear when that VM stops. To avoid rebuilding identical `Dockerfile`
contexts across samples, `inspect-capsem-sandbox` computes a 12-hex SHA-256 digest over the
`.dockerignore`-filtered build context, `Dockerfile`, build args/target, and CA fingerprint, saves
the built image with `docker save | gzip -1`, downloads it to
`~/.cache/inspect-capsem/images/<digest>.tar.gz` on the host, and loads it into subsequent sample
VMs with `docker load`. Because the guest `dockerd` runs with `"buildkit": false`, BuildKit-only
`RUN <<EOF` / `COPY <<EOF` heredocs and `RUN --mount=...` flags are lowered to classic-builder
equivalents before invoking `docker build`.

### MITM CA trust inside containers

Capsem's guest rootfs trusts the per-session MITM proxy CA at
`/usr/local/share/ca-certificates/capsem-ca.crt`, but container images brought in via `Dockerfile`
or `docker run` have their own root stores and do not trust the VM's CA by default:

1. **Build time (`patch_dockerfile_for_capsem_ca`)**: Before running `docker build`, the VM's
   `capsem-ca.crt` and a merged bundle are staged into the build context and injected into every
   external `FROM` stage: `COPY` places the CA at `/usr/local/share/ca-certificates/capsem-ca.crt`
   plus a separate merged bundle at `/usr/local/share/capsem/ca-bundle.crt` (the image's own
   `/etc/ssl/certs/ca-certificates.crt` is never replaced directly), `ENV` sets `SSL_CERT_FILE`,
   `REQUESTS_CA_BUNDLE`, `PIP_CERT`, `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`, `NODE_EXTRA_CA_CERTS`, and
   `UV_NATIVE_TLS=1`, and a best-effort `update-ca-certificates` step merges the CA into the system
   store when available.
2. **Runtime (`runtime.py` and `nss_trust.py`)**: When starting a container (`docker run` or the OCI
   `runc` fast path), the VM's CA is written to `/usr/local/share/ca-certificates/capsem-ca.crt`
   with a merged bundle at `/usr/local/share/capsem/ca-bundle.crt` (plus best-effort
   `update-ca-certificates`), and registered in the container user's NSS database
   (`sql:$HOME/.pki/nssdb`) via `certutil` or a `ctypes` fallback against `libnss3.so` so headless
   Chromium and Playwright trust HTTPS responses alongside OpenSSL, Python, and Node.js clients.

