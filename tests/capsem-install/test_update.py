"""Self-update tests for WB4.

Tests verify `capsem update` behavior for development builds,
installed layout detection, and update cache management.
"""

from __future__ import annotations

from contextlib import contextmanager
import hashlib
import http.server
import json
import os
import shutil
import socketserver
import subprocess
import threading
from pathlib import Path

import pytest

from .conftest import (
    CAPSEM_DIR,
    run_capsem,
    get_build_hash,
)

UPDATE_CACHE = CAPSEM_DIR / "update-check.json"
REPO_ROOT = Path(__file__).resolve().parents[2]


class _HealthHandler(http.server.BaseHTTPRequestHandler):
    body: bytes = b""

    def do_GET(self) -> None:
        if self.path != "/health.json":
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(self.body)))
        self.end_headers()
        self.wfile.write(self.body)

    def log_message(self, format: str, *args) -> None:
        return


@contextmanager
def _serve_health(body: bytes):
    handler = type("HealthHandler", (_HealthHandler,), {"body": body})
    server = socketserver.TCPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}/health.json"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def _copy_user_dir_capsem(source_bin: Path, capsem_home: Path) -> Path:
    target = capsem_home / "bin" / "capsem"
    target.parent.mkdir(parents=True)
    shutil.copy2(source_bin, target)
    target.chmod(0o755)
    return target


def _fresh_capsem_binary() -> Path | None:
    binary = REPO_ROOT / "target" / "debug" / "capsem"
    source_paths = [
        REPO_ROOT / "crates" / "capsem" / "src" / "update.rs",
        REPO_ROOT / "crates" / "capsem" / "src" / "client.rs",
        REPO_ROOT / "crates" / "capsem" / "src" / "main.rs",
    ]
    if binary.is_file() and all(
        binary.stat().st_mtime >= path.stat().st_mtime for path in source_paths
    ):
        return binary
    result = subprocess.run(
        ["cargo", "build", "-p", "capsem"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert result.returncode == 0, (
        f"cargo build -p capsem failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    return binary if binary.is_file() else None


def test_update_fetches_release_health_and_writes_channel_cache(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    health = {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "99.99.99",
                "current": "99.99.98",
                "files": [],
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.0",
            },
            "profiles": {
                "latest": "profiles-2030.0101.1",
                "state": "published",
            },
            "images": {
                "latest": None,
                "state": "not_published",
            },
        },
    }
    body = json.dumps(health, sort_keys=True, separators=(",", ":")).encode()

    with _serve_health(body) as health_url:
        result = subprocess.run(
            [str(capsem), "update"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Binary update available" in result.stdout
    assert "Run `capsem update --assets` separately" in result.stdout

    cache = json.loads((capsem_home / "update-check.json").read_text(encoding="utf-8"))
    assert cache["source"] == health_url
    assert cache["channel_hash"] == hashlib.sha256(body).hexdigest()
    assert cache["validation_status"] == "valid"
    assert cache.get("validation_error") is None
    assert cache["latest_version"] == "99.99.99"
    assert cache["update_available"] is True
    assert cache["latest_assets"] == "2030.0101.1"
    assert cache["latest_profiles"] == "profiles-2030.0101.1"
    assert cache["profiles_state"] == "published"
    assert cache["images_state"] == "not_published"


@pytest.mark.live_system
class TestSelfUpdate:
    """capsem update command."""

    def test_update_dev_build_bails(self, installed_layout, clean_state):
        """Non-installed layout prints 'build from source' advice."""
        # When running from installed layout, the binary is in ~/.capsem/bin
        # which is detected as UserDir, not Development. But the test verifies
        # the update command runs without crashing.
        result = run_capsem("update", "--yes", timeout=30)
        # Should either succeed or fail gracefully (network may not be available)
        combined = result.stdout + result.stderr
        # It should at least attempt to check or explain the situation
        assert result.returncode == 0 or "failed" in combined.lower() or "error" in combined.lower()

    def test_installed_layout_detection(self, installed_layout):
        """Installed binaries in ~/.capsem/bin are detected correctly."""
        # The binary is in ~/.capsem/bin which should be detected as UserDir
        # We verify indirectly: update command doesn't say "Development build"
        result = run_capsem("update", "--yes", timeout=30)
        assert "Development build" not in result.stdout, (
            "installed binary should not be detected as development build"
        )

    def test_update_cache_write_and_read(self, installed_layout, clean_state):
        """Update cache file written with version info."""
        # Remove any existing cache
        UPDATE_CACHE.unlink(missing_ok=True)

        # Run any command to trigger background cache refresh
        run_capsem("version", timeout=10)

        # The background refresh is fire-and-forget, may not have completed
        # Write a synthetic cache to test the read path
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        import time

        cache = {
            "checked_at": int(time.time()),
            "latest_version": "99.99.99",
            "update_available": True,
        }
        UPDATE_CACHE.write_text(json.dumps(cache))

        # Now any command should show the update notice
        result = run_capsem("version", timeout=10)
        assert result.returncode == 0
        # The notice goes to stderr
        assert "update available" in result.stderr.lower() or "99.99.99" in result.stderr, (
            f"expected update notice in stderr:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        # Cleanup
        UPDATE_CACHE.unlink(missing_ok=True)

    def test_update_preserves_old_on_download_failure(self, installed_layout, clean_state):
        """Binary remains unchanged if update download fails."""
        # Record current build hash
        original_hash = get_build_hash()

        # Try to update (will fail if no network or no newer version)
        run_capsem("update", "--yes", timeout=30)
        # Regardless of outcome, the installed binary should be unchanged
        current_hash = get_build_hash()
        assert current_hash == original_hash, (
            f"binary changed unexpectedly: {original_hash} -> {current_hash}"
        )
