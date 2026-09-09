"""Managed cache sockets must work under the real macOS network boundary."""

import subprocess
import sys
import uuid
from pathlib import Path

import pytest
from capsem_builder.gate import cachelayout, config, sandbox

ROOT = Path(__file__).resolve().parents[3]
CONFIG = config.load(ROOT)


@pytest.mark.parametrize("stage", ["rust-sccache", "test-temp"])
def test_cache_socket_policy_tracks_the_cache_authority(stage: str) -> None:
    directory = cachelayout.stage_path(CONFIG, stage).resolve()
    profile = sandbox.profile(CONFIG, report=False)
    assert f'(allow network* (subpath "{directory}"))' in profile
    assert "(deny network*)" in profile


@pytest.mark.skipif(sys.platform != "darwin", reason="Seatbelt is macOS")
@pytest.mark.parametrize("stage", ["rust-sccache", "test-temp"])
def test_managed_cache_socket_really_binds(stage: str, tmp_path: Path) -> None:
    if sandbox.active(CONFIG):
        pytest.skip("Seatbelt cannot be nested")
    directory = cachelayout.stage_path(CONFIG, stage)
    directory.mkdir(parents=True, exist_ok=True)
    endpoint = directory / f"probe-{uuid.uuid4().hex[:8]}.sock"
    profile = tmp_path / "cache.sb"
    profile.write_text(sandbox.profile(CONFIG, report=False))
    script = "import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()"
    try:
        result = subprocess.run(
            sandbox.wrap(CONFIG, profile, (sys.executable, "-c", script, str(endpoint))),
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 0, result.stderr
    finally:
        endpoint.unlink(missing_ok=True)
