# Capsem Inspect AI Sandbox Extension (`inspect-capsem-sandbox`)

`inspect-capsem-sandbox` registers a `capsem` [`SandboxEnvironment`](https://inspect.aisi.org.uk/sandboxing.html)
for [Inspect AI](https://inspect.aisi.org.uk/) backed by isolated Capsem micro-VMs.

| What | Name |
| --- | --- |
| Distribution (`pip install`) | `inspect-capsem-sandbox` |
| Python import package | `inspect_capsem` |
| Inspect sandbox type | `capsem` |
| Repository directory | `integrations/inspect-ai` |

## Installation

`inspect-capsem-sandbox` depends on [`capsem`](../../sdk/python/README.md) (`>=0.6.3`), `inspect-ai`,
`pydantic`, and `pyyaml`, and drives the Capsem HTTP gateway (`capsem-service`) through the
Python `capsem` SDK. It requires SDK APIs newer than the released `capsem 0.6.3`:
those already on `main` (`EXEC_TIMEOUT_CEILING_SECS`, `GATEWAY_REQUEST_BUDGET_SECS`,
`decode_exec_output`, and OCI container fields on `Hypervisor.create`) plus `#286` (streaming
of large binary bodies), `#287` (`CREATE_READY_SECS` and the `Hypervisor.create`
`request_timeout`), and `Hypervisor.vm(id=...)` foreign VM attachment; the `capsem>=0.6.3`
floor in `integrations/inspect-ai/pyproject.toml` must be bumped to the next published `capsem`
release once cut. Install from the GitHub repository, pinned to the same branch, tag, or
commit revision `<rev>` so the extension and the SDK match:

```bash
pip install \
  "capsem @ git+https://github.com/google/capsem.git@<rev>#subdirectory=sdk/python" \
  "inspect-capsem-sandbox @ git+https://github.com/google/capsem.git@<rev>#subdirectory=integrations/inspect-ai"
```

With `uv`:

```bash
uv add \
  "capsem @ git+https://github.com/google/capsem.git@<rev>#subdirectory=sdk/python" \
  "inspect-capsem-sandbox @ git+https://github.com/google/capsem.git@<rev>#subdirectory=integrations/inspect-ai"
```

Once installed, `inspect_ai` discovers the `capsem` sandbox environment through
its `[project.entry-points.inspect_ai]` entry point.

## Quickstart

Run any Inspect evaluation in a Capsem VM sandbox:

```bash
inspect eval task.py --sandbox capsem
```

Or configure the sandbox in a `@task` definition (`execution_mode="vm"` runs directly in a
Capsem micro-VM; `execution_mode="container"` runs inside an OCI workload container or nested
Docker container built from an `image`, `dockerfile`, or single-service `compose_file`):

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

A string sandbox config is also accepted: a `Dockerfile` / `Containerfile` path, a
`compose.yaml` / `docker-compose.yml` path, or a container image reference
(`"python:3.12-slim"`).

## Documentation

See [`web/docs/src/content/docs/usage/inspect-ai.md`](../../web/docs/src/content/docs/usage/inspect-ai.md) for:

- `vm` vs. `container` execution modes, single-service Docker Compose support, and `CapsemSandboxConfig` fields
- Gateway discovery (`CAPSEM_GATEWAY_URL`, `CAPSEM_GATEWAY_TOKEN`, `CAPSEM_RUN_DIR`, `CAPSEM_HOME`) and `INSPECT_CAPSEM_*` image-cache / staging environment variables
- Execution timeouts, output limits (`OutputLimitExceededError`), `write_file` permissions, and `SIGINT` / `inspect sandbox cleanup capsem` lifecycle handling
- In-guest `Dockerfile` builds, host content-addressed image caching, and MITM CA trust injection inside containers

For the underlying Python gateway client, see [`sdk/python/README.md`](../../sdk/python/README.md).

