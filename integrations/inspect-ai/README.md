# Inspect AI integration (`inspect-capsem-sandbox`)

`inspect-capsem-sandbox` (`inspect_capsem`) registers the `capsem` [`SandboxEnvironment`](https://inspect.aisi.org.uk/sandboxing.html) for [Inspect AI](https://inspect.aisi.org.uk/), running evaluations inside isolated Capsem micro-VMs via the Python [`capsem`](../../sdk/python/README.md) SDK.

Use a Capsem 0.7.1 or newer service and Python SDK. The extension needs the VM
labels and typed errors introduced with that SDK cohort. It is not published
to PyPI yet.

From a repository checkout:

```bash
uv sync --project integrations/inspect-ai --frozen
uv run --project integrations/inspect-ai inspect eval task.py --sandbox capsem
```

See the [Inspect AI Sandbox documentation](https://docs.capsem.org/usage/inspect-ai/) for configuration (`CapsemSandboxConfig`), environment variables, and lifecycle behavior.

The default VM mode runs commands and file transfers through the public SDK.
Container mode creates an OCI workload in a Capsem VM and targets that workload
for execution and transfers. The adapter labels its ephemeral VMs and tracks
cleanup; named or persistent VMs are excluded from its leftover sweep.

Compose and Dockerfile inputs share the existing instruction scanner, final-stage
USER/WORKDIR metadata extraction, and CA patcher. Compose parsing uses an
evaluator-supplied `ComposeInputs` snapshot and explicit byte, node, and depth
limits. Parsing does not grant access to host paths or credentials.

Host builds require Docker BuildKit and an explicit operator host-build grant.
BuildKit executes original Dockerfile instructions, including heredocs; CA
patching preserves instruction bodies. The adapter stages the resulting image
through its loopback registry and supplies that registry's CA through the SDK.
The service must admit the registry source and image namespace as described in
the configuration documentation.

The package tests cover SDK lifecycle and transfer behavior, Compose/Dockerfile
parsing, installed wheel/sdist payloads, and a native Docker BuildKit fixture.
Complete Capsem VM and release acceptance remain separate qualification gates.
