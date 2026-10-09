## About

Use the Capsem SDK to create a VM, run commands, and inspect its activity from
your application.

## Installation

Requires Python 3.11 or later.

```sh
python -m pip install capsem
```

[Install Capsem](https://docs.capsem.org/getting-started/installation/) and
start it on your computer before using the SDK.

## Usage

Once Capsem is running, create a VM and run a command inside it. Use an unused
VM name in place of `sdk-demo`.

Draft: automatic local connection is pending [#344](https://github.com/google/capsem/issues/344).
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

For more, use the [SDK guides](https://docs.capsem.org/guides/sdk/) or the
[reference](https://docs.capsem.org/reference/). The generated SDK API reference
is TODO.
