# Capsem Python SDK

An async client for the Capsem HTTP gateway. It takes an explicit gateway URL
and bearer token; it does not discover services, open local service sockets,
or run host commands.

```python
from capsem import Hypervisor, VM
from capsem.models import HostLogSource, TimelineLayer

async with Hypervisor("http://127.0.0.1:19222", token, timeout=120) as hv:
    overview = await hv.info()  # health, versions, profiles, updates
    vm = await hv.create("code", name="workspace", vcpu=4, memory="8G")
    result = await vm.exec("echo hello", timeout_secs=60)
    print(result.stdout, result.exit_code)
    await vm.copy.to_vm("/hello.txt", b"hello\n")
    contents = await vm.copy.from_vm("/hello.txt")
    files = await vm.list("/")
    snapshots = await vm.snapshots.list()
    details = await vm.stats.details()
    history = await vm.history()
    timeline = await vm.timeline(layers=[TimelineLayer.EXEC, TimelineLayer.MODEL])
    logs = await hv.log(HostLogSource.SERVICE, tail=100)
    await vm.stop()

# Or select an existing VM by exactly one name or canonical ID.
async with VM("http://127.0.0.1:19222", token, name="workspace") as vm:
    info = await vm.info()  # includes AI/model/MCP, network and file information
```

Named VMs are persistent; an omitted name creates an ephemeral VM. Omitting
`vcpu` or `memory` uses the selected profile's defaults. Memory accepts a
positive integer in MB or an `M`/`G` string such as `512M` or `8G`.

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
The guest exec channel currently combines both streams in `stdout`; `stderr`
is empty. The SDK preserves this gateway behavior.

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

This initial SDK does not yet expose snapshot
creation/restoration, mounts, port exposure, or subnet management.
