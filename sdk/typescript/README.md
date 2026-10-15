## About

Use the Capsem SDK to create a VM, run commands, and inspect its activity from
your application.

## Installation

Requires Node.js 20 or later for this example.

```sh
npm install @capsem/sdk
```

[Install Capsem](https://docs.capsem.org/getting-started/installation/) and
start it on your computer before using the SDK.

## Usage

Draft: the local-connection example below depends on [SDK local access](https://github.com/google/capsem/issues/344)
and [decoded command output](https://github.com/google/capsem/issues/345). For the
currently supported gateway connection, use the [integration guide](https://docs.capsem.org/guides/developers/integrate-the-sdk/#connect-to-a-remote-installation).

Once Capsem is running, create a VM and run a command inside it. Use an unused
VM name in place of `sdk-demo`.

Use a Node.js ES module with top-level `await`.

```ts
import {Hypervisor} from '@capsem/sdk';

const capsem = new Hypervisor();
const vm = await capsem.create({name: 'sdk-demo', image: 'claude-code'});
const result = await vm.exec('echo hello', {timeout_secs: 60});
console.log(String(result));
await vm.stop();
capsem.close();
```

`name` keeps the VM for later use: stopping it preserves your work so you can
start it again or duplicate it. `image` selects a Capsem prebuilt container
image or your own OCI image when permitted by your administrator.
[Learn about image selection](https://docs.capsem.org/getting-started/how-to-use-capsem/#choose-an-image-for-your-vm).

The example runs `echo hello`, displays its output, and leaves `sdk-demo`
stopped with its work preserved.

For more, use the [SDK guides](https://docs.capsem.org/guides/developers/integrate-the-sdk/) or the
[TypeScript SDK reference](https://docs.capsem.org/sdk/typescript/).
