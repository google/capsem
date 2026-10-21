## About

Use the Capsem SDK to create a VM, run commands, and inspect its activity from
your application.

## Installation

Requires Node.js 20 or later for this example.

```sh
npm install @capsem/sdk@0.7.0
```

[Install Capsem](https://docs.capsem.org/getting-started/installation/) and
start it on your computer before using the SDK.

## Usage

`new Hypervisor()` connects to Capsem running on your computer in Node.js
without requiring you to copy a gateway URL or token.

Once Capsem is running, create a VM and run a command inside it. Use an unused
VM name in place of `sdk-demo`.

Use a Node.js ES module with top-level `await`.

```ts
import {decodeExecOutput, Hypervisor} from '@capsem/sdk';

const capsem = new Hypervisor();
const vm = await capsem.create({name: 'sdk-demo', image: 'claude-code'});
const result = await vm.exec('echo hello', {timeout_secs: 60});
console.log(new TextDecoder().decode(decodeExecOutput(result.stdout)));
await vm.stop();
capsem.close();
```

`name` keeps the VM for later use: stopping it preserves your work so you can
start it again or duplicate it. `image` selects a Capsem prebuilt container
image or your own OCI image when permitted by your administrator.
[Learn about image selection](https://docs.capsem.org/getting-started/how-to-use-capsem/#choose-an-image-for-your-vm).

The example runs `echo hello`, displays its output, and leaves `sdk-demo`
stopped with its work preserved.

Use [managed scopes](https://docs.capsem.org/guides/developers/integrate-the-sdk/#bound-temporary-work)
to remove temporary VMs automatically. Look up methods and options in the
[TypeScript SDK reference](https://docs.capsem.org/sdk/typescript/), and handle
failed commands with [Errors and recovery](https://docs.capsem.org/sdk/errors/).
