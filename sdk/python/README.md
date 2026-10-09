# Capsem Python SDK

Image catalog and prefetch use the service's admission policy:

```python
from capsem import Hypervisor

async with Hypervisor(url, token) as hv:
    catalog = await hv.images.list(refresh=False)
    for image in catalog.images:
        print(image.name, image.image, image.cached)
    pulled = await hv.images.pull("code", request_timeout=120)
    print(pulled.resolved)
```

An absent `image.image` means no compatible host version. Cache `unknown`
does not establish readiness. Pass `registry=Registry(...)` to one `pull`
when private access is needed; it is not retained for later calls. Images
share the Hypervisor's connection lifetime. Prefetch does not create a VM.

`images.list` lists the registry catalog. Its `cached` field reports the
service's local disk observation: `missing`, `partial`, `ready`, or `unknown`
when verification is pending or invalidated. `ready` describes verified local
bytes; image admission and registry freshness remain separate decisions.

The complete SDK test suite installs the built wheel and sdist into separate
clean environments and runs authenticated HTTP checks with only runtime
dependencies. The gate builds both archives before running the suite. Direct
package verification from the repository root uses the same artifacts:

```sh
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 60 -- \
  uv run --project sdk/python --frozen python -m build --no-isolation \
  --outdir cache/target/sdk/python sdk/python
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 90 -- \
  uv run --project sdk/python --frozen python -m pytest \
  sdk/python/tests/test_package_install.py --no-cov -q
```

Focused source tests do not need this package build. Clean-install checks use
the prewarmed offline dependency cache and remove their runtime environments.

An async client for the Capsem HTTP gateway. It takes an explicit gateway URL
and bearer token; it does not discover services, open local service sockets,
or run host commands.

```python
from capsem import Hypervisor, VM
from capsem.models import HostLogSource, TimelineLayer

async with Hypervisor("http://127.0.0.1:19222", token, timeout=120) as hv:
    overview = await hv.info()  # health, versions, assets, updates
    network = await hv.networks.create("private")
    vm = await hv.create(
        name="workspace",
        cpus=4,
        memory=8,
        networks=[network],
        image="docker.io/library/nginx:alpine",
        command=["nginx", "-g", "daemon off;"],
        env={"MODE": "preview"},
    )
    port = await vm.ports.open(80, authenticate=True)
    print(port.url)
    await hv.networks.logs(network, vm=vm.id)
    result = await vm.exec("echo hello", timeout_secs=60)
    print(result)
    await vm.files.write("/workspace/hello.txt", b"hello\n")
    contents = await vm.files.read("/workspace/hello.txt")
    files = await vm.files.list()
    details = await vm.stats.details()
    history = await vm.history()
    timeline = await vm.timeline(layers=[TimelineLayer.EXEC, TimelineLayer.MODEL])
    await vm.persist("saved-workspace")
    triage = await hv.debug.triage(vm_id=vm.id, since="1h")
    server = await hv.mcp.get("filesystem")
    tools = await server.tools.list()
    logs = await hv.log(HostLogSource.SERVICE, tail=100)
    await vm.ports.close(port)
    await vm.stop()

# Or select an existing VM by exactly one name or canonical ID.
async with VM("http://127.0.0.1:19222", token, name="workspace") as vm:
    info = await vm.info()  # includes AI/model/MCP, network and file information
```

Named VMs are persistent; an omitted name creates an ephemeral VM. Pass
`labels={"suite": "eval", ...}` to `create(...)` to attach advisory string
labels (`<= 64` entries; keys `1..=64` ASCII matching the VM-name rule
`[A-Za-z0-9][A-Za-z0-9_-]{0,63}`; values `<= 255` UTF-8 bytes without control
characters). Omitting `cpus` or `memory` uses the service defaults (4 CPUs,
12 GiB). Memory is a positive integer in GiB.

`hv.list()` returns a typed VM inventory (including `labels` on each
`SandboxInfo`). `hv.update()` applies the configured update. VM lifecycle
methods are `start`, `stop`, `pause`, `resume`, `delete` and
`fork(name, *, description=..., labels=...)` (inherits source labels by
default; `labels={}` clears them). A fork returns another `VM` handle. Stats
has `summary()` and `details()`.

`vm.exec(command, target=models.ExecTarget.VM)` explicitly selects VM diagnostics;
`target=models.ExecTarget.WORKLOAD` selects the OCI workload. Import `models`
from `capsem`. Omitted or `None` target keeps the service default: workload
when present, otherwise VM. A refused workload target is returned as an error;
the SDK does not switch targets or replay the command.

Objects and enums live in `capsem.models`. `HttpError` exposes the gateway's
HTTP `status` and response `body`; invalid typed responses raise Pydantic
`ValidationError`. HTTP `timeout` is the client deadline, while `exec`'s
`timeout_secs` is the command deadline sent to the gateway. Choose an HTTP
deadline long enough for the command. Mutations are never automatically retried.
Create, `start`, and `resume` use at least 230 seconds for workload readiness
and the gateway budget, retaining a larger configured client deadline.
Cancelling their coroutine cancels the request without replaying the mutation.
Printing an execution result prints its decoded stdout. `stdout_bytes` and
`stderr_bytes` preserve exact bytes regardless of whether the wire value uses
UTF-8 or base64; the exit code and typed wire fields remain available.

A VM selected by name resolves once, then retains its canonical ID. Handles
returned by `create` and `fork` share their parent's connection. Close the
owning client with `async with` or `await close()`; closing a shared VM handle
does not close sibling handles. Closing a client does not stop or delete VMs.
File import/export requires a running VM's security ledger; copying from or to
a stopped VM returns `HttpError` with status 409. Stopped workspace listing
remains available.

`await hv.restart()` returns a typed HTTP 202 acknowledgement. It requires an
idle service managed by launchd or systemd; active/starting VMs return 409 and
an unmanaged service returns 503. The gateway rotates its token on restart.
Obtain fresh credentials and create a new client explicitly; never replay the
restart call. The acknowledgement does not claim reconnection has completed.

Private networks are available through `hv.networks`; pass returned network
objects directly to `create`. `vm.networks.list()` returns its current networks;
`attach(network)` and `detach(network)` accept those typed objects. Setting
`image` creates a container workload, with `command`, `env`, and a typed
`Registry` from `capsem` as optional settings. Creation returns after HTTP
reports the workload ready; `vm.container.status()` remains
a read-only diagnostic. `vm.ports.open()` opens a plain loopback TCP port by
default. `authenticate=True` uses the browser-authentication flow and returns
its URL and bootstrap material. The SDK selects the container namespace for
container workloads and the VM namespace otherwise.
Mounts remain pending.

`hv.run(command)` executes once in a temporary VM. `hv.debug.panics()` and
`hv.debug.triage()` expose host and optional VM-ledger diagnostics, and `hv.purge()`
cleans stopped VMs. `hv.mcp` covers the MCP servers every VM runs
(settings.toml `[mcp]` with corp's laid over it): `info()`, `servers()`,
`default_permission()` and `get(name)`, whose server has `tools.list()`,
`tools.call(name, arguments)` and `refresh()`. MCP calls retain native JSON
arguments/results.
