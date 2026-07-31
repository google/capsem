from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "scripts" / "serve-release-test-root.py"


def _wait_for_ready(path: Path, process: subprocess.Popen[str]) -> dict[str, str]:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if path.is_file():
            return json.loads(path.read_text(encoding="utf-8"))
        if process.poll() is not None:
            raise AssertionError(
                f"release fixture server exited early: {process.stderr.read()}"
            )
        time.sleep(0.02)
    raise AssertionError("release fixture server did not publish readiness")


def test_release_fixture_server_exposes_only_its_root_and_cleans_readiness(
    tmp_path: Path,
) -> None:
    root = tmp_path / "dist"
    root.mkdir()
    root.joinpath("manifest.json").write_bytes(b'{"channel":"local"}')
    outside = tmp_path / "outside.txt"
    outside.write_text("secret", encoding="utf-8")
    ready = tmp_path / "ready.json"
    process = subprocess.Popen(
        [
            sys.executable,
            str(SERVER),
            "--root",
            str(root),
            "--ready-file",
            str(ready),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        state = _wait_for_ready(ready, process)
        assert state["root"] == str(root.resolve())
        assert state["base_url"].startswith("http://127.0.0.1:")
        with urlopen(f"{state['base_url']}/manifest.json", timeout=2) as response:
            assert response.read() == b'{"channel":"local"}'
        try:
            urlopen(f"{state['base_url']}/../outside.txt", timeout=2)
        except HTTPError as error:
            with error:
                assert error.code == 404
        else:
            raise AssertionError("fixture server escaped its configured root")
    finally:
        process.terminate()
        process.communicate(timeout=5)

    assert not ready.exists()
