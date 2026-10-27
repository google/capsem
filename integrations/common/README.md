# Shared Capsem integration helpers (`capsem-integrations`)

`capsem-integrations` (`capsem_integrations`) provides the framework-agnostic Capsem micro-VM controller (`SdkCapsemController` over `Hypervisor.managed()`), staged file transfer (`_transfer`), command execution formatting (`_exec`), and OCI container / Compose / Dockerfile host-build helpers (`capsem_integrations.containers`) shared by [`inspect-capsem-sandbox`](../inspect-ai/README.md) and Harbor (`harbor-capsem-sandbox`). It is not published to PyPI yet.

From a repository checkout:

```bash
uv sync --project integrations/common --frozen
uv run --project integrations/common pytest
```
