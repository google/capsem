"""The packaged Codex configuration follows Capsem's image update lifecycle."""

from pathlib import Path

import tomllib


def test_codex_uses_image_updates_and_keeps_the_capsem_mcp_server():
    config = tomllib.loads(
        (Path(__file__).resolve().parents[2] / "images/codex-cli/config.toml").read_text()
    )
    assert config["check_for_update_on_startup"] is False
    assert config["mcp_servers"]["capsem"]["url"] == "http://mcp.capsem.internal/mcp"
