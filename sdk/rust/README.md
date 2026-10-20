## About

Use the Capsem SDK to create a VM, run commands, and inspect its activity from
your application.

## Installation

This example targets Capsem 0.7.1 and Rust 1.91 or later. Until the SDK is
published on crates.io, add it from the 0.7.1 source revision:

```sh
cargo add capsem-sdk --git https://github.com/google/capsem.git --rev 3c17b44f7a4c0457cb4b168df829daf293415c12
```

[Install Capsem](https://docs.capsem.org/getting-started/installation/) and
start it on your computer before using the SDK.

## Usage

`Hypervisor::local()` connects to Capsem running on your computer without
requiring you to copy a gateway URL or token.

Once Capsem is running, create a VM and run a command inside it. Use an unused
VM name in place of `sdk-demo`.

Use these statements inside your application's async context returning
`Result<(), Box<dyn std::error::Error>>`.

```rust,no_run
use capsem_sdk::{CreateOptions, Hypervisor};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let capsem = Hypervisor::local()?;
let vm = capsem.create(CreateOptions {
    name: Some("sdk-demo".into()),
    image: Some("claude-code".into()),
    ..Default::default()
}).await?;
let result = vm.exec("echo hello", Some(60)).await?;
println!("{}", String::from_utf8_lossy(&result.stdout.decode()?));
vm.stop().await?;
# Ok(())
# }
```

`name` keeps the VM for later use: stopping it preserves your work so you can
start it again or duplicate it. `image` selects a Capsem prebuilt container
image or your own OCI image when permitted by your administrator.
[Learn about image selection](https://docs.capsem.org/getting-started/how-to-use-capsem/#choose-an-image-for-your-vm).

The example runs `echo hello`, displays its output, and leaves `sdk-demo`
stopped with its work preserved.

Use [managed scopes](https://docs.capsem.org/guides/developers/integrate-the-sdk/#bound-temporary-work)
to remove temporary VMs automatically. Look up methods and options in the
[Rust SDK reference](https://docs.capsem.org/sdk/rust/), and handle failed
commands with [Errors and recovery](https://docs.capsem.org/sdk/errors/).
