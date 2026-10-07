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
from capsem_builder.gate.tools.audit import cli_surface

REPO_ROOT = Path(__file__).resolve().parents[2]
MCP_SRC = REPO_ROOT / "mcp" / "typescript" / "src"


# ---------------------------------------------------------------------------
# Mapping: MCP tool -> CLI path (space-separated), or (None, reason)
# ---------------------------------------------------------------------------

MCP_TO_CLI: dict[str, str | tuple[None, str]] = {
    # Session lifecycle
    "capsem_list": "list",
    "capsem_create": "create",
    "capsem_info": "info",
    "capsem_exec": "exec",
    "capsem_run": "run",
    "capsem_delete": "delete",
    "capsem_pause": "suspend",
    "capsem_resume": "resume",
    "capsem_persist": "persist",
    "capsem_purge": "purge",
    "capsem_fork": "fork",
    "capsem_image_list": "images",
    "capsem_image_pull": "images pull",
    "capsem_vm_logs": "logs",
    "capsem_status": "status",
    "capsem_history": "history",
    # MCP bridge
    "capsem_mcp_servers": "mcp servers",
    "capsem_mcp_tools": "mcp tools",
    "capsem_mcp_call": "mcp call",
    "capsem_mcp_refresh": "mcp refresh",
    # MCP-only: bridges / AI-caller helpers with no CLI analog
    "capsem_read_file": (
        None,
        "file I/O reserved for AI callers; CLI users drop into `capsem shell`",
    ),
    "capsem_write_file": (
        None,
        "file I/O reserved for AI callers; CLI users drop into `capsem shell`",
    ),
    "capsem_host_logs": (
        None,
        "host log reader for AI diagnostics; CLI users can inspect log files directly",
    ),
    "capsem_panics": (
        None,
        "host panic reader for AI diagnostics; CLI users read the log files directly",
    ),
    "capsem_triage": (
        None,
        "correlates host failures with a session for AI diagnostics; no CLI analog",
    ),
    "capsem_timeline": (
        None,
        "session timeline query for AI diagnostics; CLI users can inspect session DB directly",
    ),
    "capsem_list_files": (
        None,
        "structured file inventory for AI callers; CLI uses `capsem cp` or shell",
    ),
    "capsem_stats": (None, "structured VM telemetry for AI callers"),
    "capsem_stats_detail": (
        None,
        "typed VM security and activity ledgers for AI callers",
    ),
    "capsem_container_status": (None, "typed container readiness for agent workflows"),
    "capsem_port_open": (None, "typed workload port control has no CLI command yet"),
    "capsem_port_list": (None, "typed workload port control has no CLI command yet"),
    "capsem_port_close": (None, "typed workload port control has no CLI command yet"),
    "capsem_mcp_info": (None, "typed MCP readiness for AI callers"),
    "capsem_mcp_default": (None, "MCP policy inspection for AI callers"),
    "capsem_network_create": "network create",
    "capsem_network_list": "network list",
    "capsem_network_inspect": "network inspect",
    "capsem_network_delete": "network delete",
    "capsem_network_attach": "network connect",
    "capsem_network_detach": "network disconnect",
    "capsem_network_logs": "network logs",
    # Known drift -- possible cleanup candidate
    "capsem_stop": (
        None,
        "MCP-only -- CLI expresses stop via suspend (persistent) or delete (ephemeral). Consider removing.",
    ),
    "capsem_start": (None, "VM lifecycle action; CLI start controls the host service"),
}

# CLI subcommands that legitimately have no MCP tool.
CLI_ONLY: dict[str, str] = {
    "shell": "interactive terminal -- not an MCP concept",
    "restart": "reboot a persistent session; no MCP tool yet (drift candidate)",
    # Service-level / install-time -- not session-scoped, not AI-callable
    "update": "self-updater",
    "doctor": "boots a VM and runs capsem-doctor; could be MCP later",
    "completions": "shell completions generator",
    "uninstall": "system uninstaller",
    "install": "registers the LaunchAgent / systemd unit",
    "start": "start the background service daemon",
    "stop": "stop the background service daemon",
    "support-bundle": "host-side bug-report bundler; no service round-trip, not an AI concept",
    "cp": "host/session file copy convenience; MCP uses capsem_read_file/capsem_write_file",
    "version": "human CLI build metadata; MCP status reports typed gateway state",
    "assets status": "runtime asset diagnostics; MCP status includes aggregate asset health",
    "assets ensure": "local runtime asset repair; host MCP owns no asset download authority",
    # MCP sub-namespace: not every entry has a tool
}


# ---------------------------------------------------------------------------
# Source parsers
# ---------------------------------------------------------------------------

_MCP_TOOL_RE = re.compile(r"registerTool\(\s*'(?P<name>capsem_[a-z_]+)'")


def parse_mcp_tools() -> set[str]:
    """Extract tool names from the npm MCP's TypeScript registrations."""
    return {
        match.group("name")
        for path in MCP_SRC.glob("*.ts")
        for match in _MCP_TOOL_RE.finditer(path.read_text())
    }


def parse_cli_subcommands() -> set[str]:
    """Use the fail-closed public-surface owner, including external Args enums."""
    return set(cli_surface.capsem_cli_surface())


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_cli_inventory_includes_external_and_nested_command_owners():
    assert {"images", "images pull", "network list", "assets status"} <= parse_cli_subcommands(), (
        "CLI parity must see typed-argument namespaces and nested owners, not only flattened enums."
    )


def test_every_mcp_tool_is_declared():
    """Every npm MCP tool must be listed in MCP_TO_CLI."""
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

    declared_targets = {v for v in MCP_TO_CLI.values() if isinstance(v, str)} | set(
        CLI_ONLY
    )

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
