"""CLI <-> MCP parity.

Fails when a CLI subcommand exists without a corresponding MCP tool (or vice
versa) unless explicitly excluded with a reason. This is the guardrail that
would have caught us shipping capsem_image_* MCP tools after the CLI
dropped the image concept.

Refinement policy: when either surface legitimately diverges, update the
mapping below with a one-liner reason. No silent drift.
"""

import re
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
MCP_SRC = REPO_ROOT / "crates" / "capsem-mcp" / "src" / "main.rs"
CLI_SRC = REPO_ROOT / "crates" / "capsem" / "src" / "main.rs"


# ---------------------------------------------------------------------------
# Mapping: MCP tool -> CLI path (space-separated), or (None, reason)
# ---------------------------------------------------------------------------

MCP_TO_CLI: dict[str, str | tuple[None, str]] = {
    # Session lifecycle
    "capsem_list":      "list",
    "capsem_create":    "create",
    "capsem_info":      "info",
    "capsem_exec":      "exec",
    "capsem_run":       "run",
    "capsem_delete":    "delete",
    "capsem_suspend":   "suspend",
    "capsem_resume":    "resume",
    "capsem_persist":   "persist",
    "capsem_purge":     "purge",
    "capsem_fork":      "fork",
    "capsem_vm_logs":   "logs",
    "capsem_version":   "version",

    # MCP bridge
    "capsem_mcp_servers": "mcp servers",
    "capsem_mcp_tools":   "mcp tools",
    "capsem_mcp_call":    "mcp call",

    # MCP-only: bridges / AI-caller helpers with no CLI analog
    "capsem_read_file":       (None, "file I/O reserved for AI callers; CLI users drop into `capsem shell`"),
    "capsem_write_file":      (None, "file I/O reserved for AI callers; CLI users drop into `capsem shell`"),
    "capsem_inspect":         (None, "SQL query tool for AI callers; CLI users `sqlite3` the session DB directly"),
    "capsem_inspect_schema":  (None, "paired with capsem_inspect; AI callers need schemas before querying"),
    "capsem_service_logs":    (None, "no CLI equivalent yet -- candidate for `capsem service logs`"),
    "capsem_panics":          (None, "host diagnostic triage tool; no CLI equivalent yet"),
    "capsem_triage":          (None, "host diagnostic triage summary; no CLI equivalent yet"),
    "capsem_host_logs":       (None, "host log reader for AI diagnostics; CLI users can inspect log files directly"),
    "capsem_timeline":        (None, "session timeline query for AI diagnostics; CLI users can inspect session DB directly"),

    # Known drift -- possible cleanup candidate
    "capsem_stop":            (None, "MCP-only -- CLI expresses stop via suspend (persistent) or delete (ephemeral). Consider removing."),
}

# CLI subcommands that legitimately have no MCP tool.
CLI_ONLY: dict[str, str] = {
    "shell":        "interactive terminal -- not an MCP concept",
    "restart":      "reboot a persistent session; no MCP tool yet (drift candidate)",
    "history":      "host-side command audit view; no MCP tool yet (drift candidate)",

    # Service-level / install-time -- not session-scoped, not AI-callable
    "update":       "self-updater",
    "doctor":       "boots a VM and runs capsem-doctor; could be MCP later",
    "completions":  "shell completions generator",
    "uninstall":    "system uninstaller",
    "install":      "registers the LaunchAgent / systemd unit",
    "status":       "service + asset health; prints a human table",
    "start":        "start the background service daemon",
    "stop":         "stop the background service daemon",
    "support-bundle": "host-side bug-report bundler; no service round-trip, not an AI concept",
    "cp":           "host/session file copy convenience; MCP uses capsem_read_file/capsem_write_file",

    # MCP sub-namespace: not every entry has a tool
    "mcp policy":   "read-only policy dump; AI callers don't need it",
    "mcp refresh":  "forces tool re-discovery; AI callers re-list directly",
}


# ---------------------------------------------------------------------------
# Source parsers
# ---------------------------------------------------------------------------

_MCP_TOOL_RE = re.compile(r"#\[tool\((?P<body>.*?)\)\]", re.S)
_MCP_TOOL_NAME_RE = re.compile(r'name\s*=\s*"(?P<name>capsem_[a-z_]+)"')


def parse_mcp_tools() -> set[str]:
    """Extract tool names from #[tool(name = "...")] attributes."""
    src = MCP_SRC.read_text()
    names = set()
    for attr in _MCP_TOOL_RE.finditer(src):
        name = _MCP_TOOL_NAME_RE.search(attr.group("body"))
        if name:
            names.add(name.group("name"))
    return names


def _parse_subcommand_variants(src: str, enum_name: str) -> list[str]:
    """Pull variant names (kebab-cased) from a `enum <Name> { ... }` block."""
    m = re.search(rf"enum {enum_name} \{{(?P<body>.*?)^\}}", src, re.S | re.M)
    assert m, f"could not find `enum {enum_name}` in capsem/src/main.rs"
    body = m.group("body")
    # Strip attributes and doc comments; find CamelCase variant identifiers at
    # top level of the enum block (ignoring inner struct fields).
    # Variants appear as `Name {` or `Name,` or `Name` at line start (after ws).
    variants = []
    for line in body.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(("//", "#[", "/*", "*")):
            continue
        vm = re.match(r"([A-Z][A-Za-z0-9]*)\s*[\{,]?\s*$", stripped)
        if vm:
            variants.append(_camel_to_kebab(vm.group(1)))
    return variants


def _camel_to_kebab(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()


def parse_cli_subcommands() -> set[str]:
    """Return the full set of CLI subcommand paths, space-separated.

    SessionCommands and MiscCommands are `#[command(flatten)]` -- their
    variants become top-level subcommands. McpCommands is nested under
    `capsem mcp <variant>`.
    """
    src = CLI_SRC.read_text()
    top = set(_parse_subcommand_variants(src, "SessionCommands"))
    top.update(_parse_subcommand_variants(src, "MiscCommands"))
    nested = {f"mcp {v}" for v in _parse_subcommand_variants(src, "McpCommands")}
    return top | nested


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_every_mcp_tool_is_declared():
    """Every #[tool] in capsem-mcp must be listed in MCP_TO_CLI."""
    actual = parse_mcp_tools()
    declared = set(MCP_TO_CLI)
    missing = actual - declared
    stale = declared - actual
    assert not missing, (
        f"MCP tools not declared in MCP_TO_CLI: {sorted(missing)}. "
        "Add them with their CLI path or (None, reason)."
    )
    assert not stale, (
        f"MCP_TO_CLI references tools that no longer exist: {sorted(stale)}. "
        "Remove these entries."
    )


def test_every_cli_subcommand_is_declared():
    """Every CLI subcommand must map from some MCP tool OR be in CLI_ONLY."""
    actual = parse_cli_subcommands()

    declared_targets = {
        v for v in MCP_TO_CLI.values() if isinstance(v, str)
    } | set(CLI_ONLY)

    missing = actual - declared_targets
    stale = declared_targets - actual
    assert not missing, (
        f"CLI subcommands with no MCP tool and not in CLI_ONLY: {sorted(missing)}. "
        "Either add an MCP tool or add to CLI_ONLY with a reason."
    )
    assert not stale, (
        f"Mapping references CLI subcommands that do not exist: {sorted(stale)}. "
        "Update MCP_TO_CLI or CLI_ONLY."
    )


@pytest.mark.parametrize(
    "tool,target",
    [(t, v) for t, v in MCP_TO_CLI.items() if isinstance(v, str)],
)
def test_mcp_tool_cli_target_exists(tool: str, target: str):
    """The CLI path declared for each MCP tool must actually exist in clap."""
    cli = parse_cli_subcommands()
    assert target in cli, (
        f"{tool} is declared to map to `capsem {target}`, but that subcommand "
        f"does not exist in capsem CLI."
    )
