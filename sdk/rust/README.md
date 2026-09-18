# Capsem Rust SDK

Typed async clients for the HTTP gateway. Supply its URL and bearer token
explicitly; the SDK does not discover local services or run host commands.

```rust,no_run
use capsem_sdk::{CreateOptions, Hypervisor, LogOptions, PortOptions, Result, TriageOptions, VmSelector};
use capsem_sdk::models::HostLogSource;
use std::collections::HashMap;
use std::time::Duration;

async fn example(url: &str, token: &str) -> Result<()> {
    let hv = Hypervisor::new(url, token)?.with_timeout(Duration::from_secs(120))?;
    println!("{:?}", hv.info().await?); // health, version, profiles, updates
    let profile = hv.profiles().list().await?.remove(0);
    let network = hv.networks().create("private").await?;
    let vm = hv.create(CreateOptions {
        name: Some("work".into()), cpus: Some(4), memory: Some(8),
        networks: vec![network.clone()],
        env: Some(HashMap::from([("MODE".into(), "preview".into())])),
        image: Some("docker.io/library/nginx:alpine".into()),
        command: vec!["nginx".into(), "-g".into(), "daemon off;".into()],
        ..Default::default()
    }).await?;
    let port = vm.ports().open_with(80, PortOptions { host: 0, authenticate: true }).await?;
    println!("submit the bootstrap token by POST to {}", port.url.as_deref().unwrap_or(""));
    hv.networks().logs(&network, Default::default()).await?;
    let result = vm.exec("echo hello", Some(60)).await?;
    println!("{} (exit {})", result.stdout.data, result.exit_code);
    vm.files().write("/workspace/hello.txt", b"hello\n".to_vec()).await?;
    let contents = vm.files().read("/workspace/hello.txt").await?;
    assert_eq!(contents, b"hello\n");
    vm.files().list("", None).await?;
    vm.snapshots().list().await?;
    vm.stats().details().await?;
    vm.persist("saved-workspace").await?;
    hv.debug().triage(TriageOptions { vm_id: vm.id().map(str::to_owned), since: Some("1h".into()), ..Default::default() }).await?;
    let server = hv.profiles().mcp(&profile).get("filesystem").await?;
    server.tools().list().await?;
    hv.log(HostLogSource::Service, LogOptions { tail: Some(100), ..Default::default() }).await?;
    vm.ports().close(&port).await?;
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

Omitted CPU and memory values retain the profile defaults. Omit `profile` for the
profile the gateway's catalog names as its default (read once from
`GET /status` and cached on the handle); select another by assigning a
`ProfileSummary` returned by `hv.profiles().list().await?` to the options. Names
create persistent VMs; omitted or empty names create ephemeral VMs. Memory is
measured in GiB.

VM controls are `start`, `stop`, `pause`, `resume`, `delete`, and
`fork(name, description)`. Queries include `log(LogOptions)`,
`history(HistoryOptions)`, `timeline(TimelineOptions)`,
and `files().list/read/write/history`.
Snapshots support `list()` and `status()`; stats support `summary()` and
`details()`. File reads and writes preserve bytes and require a running VM's
security ledger. `hv.update()` applies the configured update.

Responses and enums reuse `capsem_sdk::models` (the gateway's `capsem-api`
DTOs). `operations` exposes the generated request functions and parameter
structs. `Error::Http` retains status and response bytes; malformed JSON
returns `Error::Json`. HTTP requests default to 30 seconds and never follow
redirects or retry automatically. `with_timeout` sets this handle's HTTP
deadline; `exec`'s optional `timeout_secs` sets the guest command deadline.
Choose an HTTP deadline long enough for the command.
The guest exec channel preserves separate `stdout` and `stderr` lanes. Each is
a typed `ExecOutput`; `decode()` returns the exact bytes.

`hv.restart().await?` returns a typed HTTP 202 acknowledgement. It requires an
idle service managed by launchd or systemd; active/starting VMs return 409 and
an unmanaged service returns 503. The gateway rotates its token on restart.
Obtain fresh credentials and construct a new client explicitly; never replay
the restart call. Acceptance does not claim reconnection has completed.

`hv.networks()` provides typed create/list/inspect/delete and cursor-based audit
logs; pass returned network objects directly to creation. VM membership uses
`vm.networks().list/attach/detach` with typed network objects.
`image` selects a container workload, with command, environment, and a typed
`Registry` alongside it. Creation returns after HTTP reports the workload
ready; `vm.container().status()` remains a read-only diagnostic. `vm.ports()`
opens a plain loopback port by default and uses browser authentication when
requested. The SDK infers the VM or container target. Typed ports can be listed
and closed without exposing wire request enums.

`hv.run(command, options)` executes once in a temporary VM. `hv.debug().panics()`,
`hv.debug().triage()` and `hv.purge()` expose diagnostics and cleanup. `hv.profiles()`
provides typed profile and MCP discovery, refresh, permissions and tool calls;
tool arguments and results retain their native JSON shape.
