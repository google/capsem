# capsem

The `capsem` command-line client. Connects to the `capsem-service` daemon over
a Unix domain socket at `~/.capsem/run/service.sock` and drives VM sessions
(`create`, `shell`, `resume`, `exec`, `run`, `list`, ...), the MCP registry
(`capsem mcp ...`), and service/system commands (`install`, `setup`, `status`).

See <https://capsem.org/usage/cli/> for the full reference and
<https://capsem.org/getting-started/> for installation.
