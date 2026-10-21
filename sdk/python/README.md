## About

Use the Capsem SDK to create a VM, run commands, and inspect its activity from
your application.

## Installation

Requires Python 3.11 or later. This example targets Capsem 0.7.1. Until its
Python package is published, install the SDK from the 0.7.1 source revision;
PyPI's `capsem` 0.3.2 does not provide this API.

```sh
python -m pip install "capsem @ git+https://github.com/google/capsem.git@3c17b44f7a4c0457cb4b168df829daf293415c12#subdirectory=sdk/python"
```

[Install Capsem](https://docs.capsem.org/getting-started/installation/) and
start it on your computer before using the SDK.

## Usage

`Hypervisor()` connects to Capsem running on your computer without requiring
you to copy a gateway URL or token.

Once Capsem is running, create a VM and run a command inside it. Use an unused
VM name in place of `sdk-demo`.

Run these direct-`await` statements in a notebook or IPython.

```python
from capsem import Hypervisor

capsem = Hypervisor()
vm = await capsem.create(name="sdk-demo", image="claude-code")
result = await vm.exec("echo hello", timeout_secs=60)
print(result)
await vm.stop()
await capsem.close()
```

`name` keeps the VM for later use: stopping it preserves your work so you can
start it again or duplicate it. `image` selects a Capsem prebuilt container
image or your own OCI image when permitted by your administrator.
[Learn about image selection](https://docs.capsem.org/getting-started/how-to-use-capsem/#choose-an-image-for-your-vm).

The example runs `echo hello`, displays its output, and leaves `sdk-demo`
stopped with its work preserved.

Pass `labels={"suite": "eval"}` to `create(...)` to attach advisory metadata.
VM inventory includes those labels. Forks inherit them by default;
`fork(name, labels={})` clears them and a nonempty map replaces them.

Use [managed scopes](https://docs.capsem.org/guides/developers/integrate-the-sdk/#bound-temporary-work)
to remove temporary VMs automatically. Look up methods and options in the
[Python SDK reference](https://docs.capsem.org/sdk/python/), and handle failed
commands with [Errors and recovery](https://docs.capsem.org/sdk/errors/).
