"""Each official image boots on the Capsem runtime and its agent runs.

Image CI, not trunk: it needs the images built (`images/`), exported as OCI
layouts with `docker save` into $CAPSEM_OFFICIAL_IMAGE_LAYOUTS/<name>/. The
test serves each layout from a local TLS registry and runs the agent's own
version command in a `capsem run --image` session, as the image's
unprivileged user.
"""

import os
import subprocess
from pathlib import Path

import pytest

from tests.fixtures.oci.registry import layout_registry
from tests.ironbank.kingslanding.test_run import command, environment, service

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
