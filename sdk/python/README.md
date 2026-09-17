# Capsem Python SDK

An async client for the Capsem HTTP gateway. It takes an explicit gateway URL
and bearer token; it does not discover services, open local service sockets,
or run host commands.

```python
from capsem import Hypervisor, VM
from capsem.models import HostLogSource, TimelineLayer

async with Hypervisor("http://127.0.0.1:19222", token, timeout=120) as hv:
    overview = await hv.info()  # health, versions, profiles, updates
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
    await hv.networks.logs(network.id, vm=vm.id)
    result = await vm.exec("echo hello", timeout_secs=60)
    print(result)
    await vm.copy.to_vm("/hello.txt", b"hello\n")
    contents = await vm.copy.from_vm("/hello.txt")
    files = await vm.list("/")
    snapshots = await vm.snapshots.list()
    details = await vm.stats.details()
    history = await vm.history()
    timeline = await vm.timeline(layers=[TimelineLayer.EXEC, TimelineLayer.MODEL])
    await vm.persist("saved-workspace")
    triage = await hv.triage(vm_id=vm.id, since="1h")
    tools = await hv.profiles.mcp("code").tools("filesystem")
    logs = await hv.log(HostLogSource.SERVICE, tail=100)
    await vm.ports.close(port)
    await vm.stop()

# Or select an existing VM by exactly one name or canonical ID.
async with VM("http://127.0.0.1:19222", token, name="workspace") as vm:
    info = await vm.info()  # includes AI/model/MCP, network and file information
```

Named VMs are persistent; an omitted name creates an ephemeral VM. Omitting
`profile` selects Capsem's standard `code` profile. To select another profile,
pass an object returned by `await hv.profiles.list()` to `create(profile=...)`
or `run(profile=...)`. Omitting `cpus` or `memory` uses the selected profile's
defaults. Memory is a positive integer in GiB.

`hv.list()` returns a typed VM inventory. `hv.update()` applies the configured
update. VM lifecycle methods are `start`, `stop`, `pause`, `resume`, `delete`
and `fork(name)`. A fork returns another `VM` handle. Snapshot inspection uses
`vm.snapshots.list()` and `status()`; `vm.changes(checkpoint)` compares workspace
paths against an existing checkpoint. Stats has `summary()` and `details()`.

Objects and enums live in `capsem.models`. `HttpError` exposes the gateway's
HTTP `status` and response `body`; invalid typed responses raise Pydantic
`ValidationError`. HTTP `timeout` is the client deadline, while `exec`'s
`timeout_secs` is the command deadline sent to the gateway. Choose an HTTP
deadline long enough for the command. Mutations are never automatically retried.
Printing an execution result prints its decoded stdout. `stdout_bytes` and
`stderr_bytes` preserve exact bytes regardless of whether the wire value uses
UTF-8 or base64; the exit code and typed wire fields remain available.

A VM selected by name resolves once, then retains its canonical ID. Handles
returned by `create` and `fork` share their parent's connection. Close the
owning client with `async with` or `await close()`; closing a shared VM handle
does not close sibling handles. Closing a client does not stop or delete VMs.
File import/export requires a running VM's security ledger; copying from or to
a stopped VM returns `HttpError` with status 409. Stopped workspace listing and
snapshot comparisons remain available.

`await hv.restart()` returns a typed HTTP 202 acknowledgement. It requires an
idle service managed by launchd or systemd; active/starting VMs return 409 and
an unmanaged service returns 503. The gateway rotates its token on restart.
Obtain fresh credentials and create a new client explicitly; never replay the
restart call. The acknowledgement does not claim reconnection has completed.

Private networks are available through `hv.networks`; pass returned network
objects directly to `create`. Setting `image` creates a container workload,
with `command`, `env`, and a typed `Registry` from `capsem` as optional settings. Creation
returns after HTTP reports the workload ready; `vm.container.status()` remains
a read-only diagnostic. `vm.ports.open()` opens a plain loopback TCP port by
default. `authenticate=True` uses the browser-authentication flow and returns
its URL and bootstrap material. The SDK selects the container namespace for
container workloads and the VM namespace otherwise.
Snapshot creation/restoration and mounts remain pending.

`hv.run(command)` executes once in a temporary VM. `hv.panics()` and
`hv.triage()` expose host and optional VM-ledger diagnostics, and `hv.purge()`
cleans stopped VMs. `hv.profiles` lists profiles and provides typed MCP server,
permission and tool discovery; MCP calls retain native JSON arguments/results.
