## About

Control Capsem VMs from your application: create a VM, run commands, pause and
resume work, duplicate a VM, and inspect its activity. The SDK connects to the
Capsem gateway using a URL and bearer token.

## Installation

Requires Python 3.11 or later.

```sh
python -m pip install capsem
```

The SDK connects to a running Capsem service. [Install Capsem](https://docs.capsem.org/getting-started/installation/)
first if you do not already have access to one.

## Usage

Once Capsem is running, supply its gateway URL and token. For a default local
installation, the URL is `http://127.0.0.1:19222` and the token is stored in
`~/.capsem/run/gateway.token`. For a remote service, use the URL and token
provided by its administrator.

Set `CAPSEM_GATEWAY_URL` and `CAPSEM_GATEWAY_TOKEN` in the environment where you
run this example. It creates a named VM, runs a command inside it, and stops it.

```python
import asyncio
import os

from capsem import Hypervisor


async def main():
    async with Hypervisor(
        os.environ["CAPSEM_GATEWAY_URL"],
        os.environ["CAPSEM_GATEWAY_TOKEN"],
    ) as capsem:
        vm = await capsem.create(name="sdk-demo", image="claude-code")
        try:
            result = await vm.exec("echo hello", timeout_secs=60)
            print(result)
        finally:
            await vm.stop()


asyncio.run(main())
```

`name` keeps the VM for later use: stopping it preserves your work, and you can
start it again or duplicate it with `vm.fork("copy")`. `image` selects the container
inside the VM; use a Capsem prebuilt image or your own OCI image when permitted
by your administrator. [Learn about image selection](https://docs.capsem.org/getting-started/how-to-use-capsem/#choose-an-image-for-your-vm).

The example prints `hello`. It leaves `sdk-demo` stopped. Closing the client
releases its connection; it does not stop or delete a VM.

Continue with the [guides](https://docs.capsem.org/guides/) for specific tasks,
or consult the [reference](https://docs.capsem.org/reference/) for exact
methods, arguments, types, and behavior. These sections are being written;
the generated Python API reference is not available yet.
