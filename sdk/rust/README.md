## About

Use the Capsem SDK to create a VM, run commands, and inspect its activity from
your application.

## Installation

Add the SDK to your Rust project:

```sh
cargo add capsem-sdk
```

[Install Capsem](https://docs.capsem.org/getting-started/installation/) and
start it on your computer before using the SDK.

## Usage

Draft: the local-connection example below depends on [SDK local access](https://github.com/google/capsem/issues/344)
and [decoded command output](https://github.com/google/capsem/issues/345). For the
currently supported gateway connection, use the [integration guide](https://docs.capsem.org/guides/developers/integrate-the-sdk/#connect-to-a-remote-installation).

Once Capsem is running, create a VM and run a command inside it. Use an unused
VM name in place of `sdk-demo`.

Use these statements inside your application's async context returning a `Result`.

```rust,ignore
use capsem_sdk::{CreateOptions, Hypervisor};

let capsem = Hypervisor::local()?;
let vm = capsem.create(CreateOptions {
    name: Some("sdk-demo".into()),
    image: Some("claude-code".into()),
    ..Default::default()
}).await?;
let result = vm.exec("echo hello", Some(60)).await?;
println!("{result}");
vm.stop().await?;
```

`name` keeps the VM for later use: stopping it preserves your work so you can
start it again or duplicate it. `image` selects a Capsem prebuilt container
image or your own OCI image when permitted by your administrator.
[Learn about image selection](https://docs.capsem.org/getting-started/how-to-use-capsem/#choose-an-image-for-your-vm).

The example runs `echo hello`, displays its output, and leaves `sdk-demo`
stopped with its work preserved.

For more, use the [SDK guides](https://docs.capsem.org/guides/developers/integrate-the-sdk/) or the
[Rust SDK reference](https://docs.capsem.org/sdk/rust/).
