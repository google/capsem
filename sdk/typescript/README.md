# Capsem TypeScript SDK

The first slice provides the gateway's generated models and runtime validation.
The async `Hypervisor` and `VM` facade is being built on this contract.

```ts
import {HostLogSource, type ExecRequest} from '@capsem/sdk';

const source = HostLogSource.SERVICE;
const request: ExecRequest = {command: 'uname -a'};
```

Models use runtime enums, exact optional properties, and distinct nullable
values. Runtime validators reject wrong primitive types, unknown enum values,
and integers that JavaScript cannot represent safely. Generated source is
included in strict compilation, lint, coverage, drift and module-size checks.

Run `pnpm install --frozen-lockfile`, then `pnpm lint`, `pnpm check`,
`pnpm test`, and `pnpm build` in this directory. The fast gate also builds the
package tarball and verifies generation against `sdk/specification/openapi.json`.
