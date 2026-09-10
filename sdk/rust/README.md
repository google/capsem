# Capsem Rust SDK

Typed async clients for the HTTP gateway. Supply its URL and bearer token
explicitly; the SDK does not discover local services or run host commands.

```rust,no_run
use capsem_sdk::{CreateOptions, Hypervisor, LogOptions, Result, VmSelector};
use capsem_sdk::models::HostLogSource;
use std::time::Duration;

async fn example(url: &str, token: &str) -> Result<()> {
    let hv = Hypervisor::new(url, token)?.with_timeout(Duration::from_secs(120))?;
    println!("{:?}", hv.info().await?); // health, version, profiles, updates
    let vm = hv.create("code", CreateOptions {
        name: Some("work".into()), vcpu: Some(4), memory: Some("8G".parse()?),
        ..Default::default()
    }).await?;
    let result = vm.exec("echo hello", Some(60)).await?;
    println!("{} (exit {})", result.stdout, result.exit_code);
    vm.copy().to_vm("/hello.txt", b"hello\n".to_vec()).await?;
    let contents = vm.copy().from_vm("/hello.txt").await?;
    assert_eq!(contents, b"hello\n");
    vm.list("/", None).await?;
    vm.snapshots().list().await?;
    vm.stats().details().await?;
    hv.log(HostLogSource::Service, LogOptions { tail: Some(100), ..Default::default() }).await?;
    vm.stop().await?;

    let existing = hv.vm(VmSelector::Name("work".into()))?;
    println!("{:?}", existing.info().await?); // AI/model/MCP, network and files
    Ok(())
}
```

`VM::new(url, token, VmSelector::Id(id))` creates an independent handle.
Name selection resolves once through the gateway; concurrent cloned handles
share that lookup. Missing or ambiguous names return `Error::VmLookup`.
Clones, created VMs and forks share the HTTP connection pool. Dropping a handle
does not invalidate other handles. Dropping a request future cancels its HTTP
request; the gateway may already have accepted a mutation.

Omitted CPU and memory values retain the profile defaults. Names create
persistent VMs; omitted or empty names create ephemeral VMs. Memory accepts
`Memory::Megabytes`, `Memory::Gigabytes` or parsed `M`/`G` strings.

VM controls are `start`, `stop`, `pause`, `resume`, `delete`, and
`fork(name, description)`. Queries include `log(LogOptions)`,
`history(HistoryOptions)`, `timeline(TimelineOptions)`,
`changes(checkpoint, PageOptions)`, and `list(path, depth)`.
Snapshots support `list()` and `status()`; stats support `summary()` and
`details()`. Copy transfers single-file bytes and requires a running VM's
security ledger. `hv.update()` applies the configured update.

Responses and enums reuse `capsem_sdk::models` (the gateway's `capsem-api`
DTOs). `operations` exposes the generated request functions and parameter
structs. `Error::Http` retains status and response bytes; malformed JSON
returns `Error::Json`. HTTP requests default to 30 seconds and never follow
redirects or retry automatically. `with_timeout` sets this handle's HTTP
deadline; `exec`'s optional `timeout_secs` sets the guest command deadline.
Choose an HTTP deadline long enough for the command.
The guest exec channel currently combines both streams in `stdout`; `stderr`
is empty. The SDK preserves this gateway behavior.

`hv.restart().await?` returns a typed HTTP 202 acknowledgement. It requires an
idle service managed by launchd or systemd; active/starting VMs return 409 and
an unmanaged service returns 503. The gateway rotates its token on restart.
Obtain fresh credentials and construct a new client explicitly; never replay
the restart call. Acceptance does not claim reconnection has completed.

Explicit snapshot creation/restoration, mounts,
port exposure and subnet configuration remain outside the implemented facade.
