"""Each official image boots on the Capsem runtime and its agent runs.

Image CI, not trunk: it needs the images built (`images/`), exported as OCI
layouts with `docker save` into $CAPSEM_OFFICIAL_IMAGE_LAYOUTS/<name>/. The
test serves each layout from a local TLS registry and runs the agent's own
version command in a `capsem run --image` session, as the image's
unprivileged user.
"""

import json
import os
import subprocess
from pathlib import Path

import pytest
from helpers.service import exec_output_text

from tests.fixtures.oci.registry import layout_registry
from tests.ironbank.kingslanding.test_run import (
    command,
    console,
    create_command,
    environment,
    service,
    wait_for,
)

__all__ = ["service"]

LAYOUTS = os.environ.get("CAPSEM_OFFICIAL_IMAGE_LAYOUTS")

pytestmark = [
    pytest.mark.integration,
    pytest.mark.skipif(not LAYOUTS, reason="image CI only: set CAPSEM_OFFICIAL_IMAGE_LAYOUTS"),
]

AGENTS = {
    "dev": (["sh", "-c", "id -u && gcc --version && node --version && ollama --version"], "1000"),
    "codex-cli": (["codex", "--version"], "codex-cli 0.147.0"),
    "claude-code": (["claude", "--version"], "2.1.229"),
    "agy": (["agy-real", "--version"], "1.1.3"),
    "claude-desktop": (["dpkg-query", "-W", "claude-desktop"], "1.22209.0"),
}


MCP_URL = "http://mcp.capsem.internal/mcp"

# Where each agent reads its user-scope MCP servers. A container cannot reach
# the in-guest vsock relay, so the agent must use the internal HTTP name the
# session's proxy answers.
MCP_CONFIGS = {
    "codex-cli": "/home/capsem/.codex/config.toml",
    "claude-code": "/home/capsem/.claude.json",
    "agy": "/home/capsem/.gemini/config/mcp_config.json",
}


@pytest.mark.parametrize("name", sorted(AGENTS))
def test_the_image_boots_and_its_agent_runs(service, tmp_path, name):
    _run_in_image(service, tmp_path, name, *AGENTS[name])


@pytest.mark.parametrize("name", sorted(MCP_CONFIGS))
def test_the_images_agent_reaches_capsem_mcp_over_http(service, tmp_path, name):
    _run_in_image(service, tmp_path, name, ["cat", MCP_CONFIGS[name]], MCP_URL)


CHROMIUM_PROCESSES = (
    # Each Chromium process: its pid, its pid in every PID namespace it is in
    # (NSpid, readable without ptrace access to a non-dumpable renderer), and
    # its command line.
    'for d in /proc/[0-9]*; do c=$(tr "\\0" " " <"$d/cmdline" 2>/dev/null) || continue; '
    'case "$c" in /usr/lib/claude-desktop/claude-desktop\\ --type=*) printf "%s|%s|%s\\n" "${d#/proc/}" '
    '"$(sed -n "s/^NSpid:[[:space:]]*//p" "$d/status" | tr "\\t" " ")" "$c";; esac; done'
)


def test_claude_desktop_runs_chromium_inside_its_namespace_sandbox(service, tmp_path):
    """Claude Desktop starts under the Xpra surface's filter with Chromium's
    sandbox on: no process runs with --no-sandbox, and its zygote and renderers
    live in PID namespaces of their own, which the user-namespace sandbox
    creates and the terminal filter refuses."""
    layout = Path(LAYOUTS or "") / "claude-desktop"
    client = service.client()
    with layout_registry(tmp_path, layout, "claude-desktop") as (reference, certificate, _):
        created = subprocess.run(
            create_command(service, reference, certificate, "-n", "desktop", "--ram", "4"),
            env=environment(service),
            capture_output=True,
            timeout=900,
            check=False,
        )
        (tmp_path / "create.stderr").write_bytes(created.stderr)
        assert created.returncode == 0, created.stderr.decode(errors="replace")
        (vm,) = (
            row for row in client.get("/vms/list")["sandboxes"] if row.get("name") == "desktop"
        )
        try:
            processes = []

            # A create returns once the workload runs, so every exec enters it.
            def sandboxed():
                result = client.post(
                    f"/vms/{vm['id']}/exec",
                    {"command": CHROMIUM_PROCESSES, "timeout_secs": 30},
                    timeout=40,
                )
                assert result.get("exit_code") == 0, result
                processes[:] = [
                    line.split("|", 2) for line in exec_output_text(result).splitlines()
                ]
                return any("--type=renderer" in line for _, _, line in processes)

            wait_for(sandboxed, "Claude Desktop renderer", timeout=600)
            (tmp_path / "processes.txt").write_text("\n".join("|".join(row) for row in processes))
            status = client.get(f"/vms/{vm['id']}/container")
            (tmp_path / "container.json").write_text(json.dumps(status, indent=2))
            assert status["state"] == "running", status
            assert not any("--no-sandbox" in line for _, _, line in processes), processes
            nested = {
                kind: [
                    len(nspid.split()) > 1
                    for _, nspid, line in processes
                    if f"--type={kind}" in line
                ]
                for kind in ("zygote", "renderer")
            }
            assert all(nested["renderer"]) and nested["renderer"], processes
            assert any(nested["zygote"]), processes
            assert "No usable sandbox" not in console(service, vm["id"])
        finally:
            client.delete(f"/vms/{vm['id']}/delete")


def _run_in_image(service, tmp_path, name, argv, expected):
    layout = Path(LAYOUTS or "") / name
    with layout_registry(tmp_path, layout, name) as (reference, certificate, _):
        result = subprocess.run(
            [*command(service, reference, certificate), *argv],
            env=environment(service),
            capture_output=True,
            timeout=600,
            check=False,
        )
    (tmp_path / "stdout").write_bytes(result.stdout)
    (tmp_path / "stderr").write_bytes(result.stderr)
    output = result.stdout.decode(errors="replace")
    assert result.returncode == 0, result.stderr.decode(errors="replace") + output
    assert expected in output, output
