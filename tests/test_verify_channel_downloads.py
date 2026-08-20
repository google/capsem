"""The last publication step, proved against a throwaway channel.

`verify-release-downloads` is the final thing a release does before anyone can
install the result, and it was the only publication job with no coverage at
all. Its body was a `curl` loop inside YAML with a blake3 check written as a
Python heredoc -- unreachable from any test.

That matters because it is the step whose absence let the live stable channel
serve `status: current` for a month with three package URLs returning 404. The
check that would have caught it was itself unchecked.

Served here by the fixture server the install suite already uses, so a whole
channel is stood up on loopback and every failure mode is a real HTTP response
rather than a mock: a row that 404s, a row whose length disagrees with the
manifest, and a row whose bytes do.
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

import blake3
import pytest

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "scripts" / "serve-release-test-root.py"
VERIFY = ROOT / "scripts" / "verify-channel-downloads.py"


def _wait_for_ready(path: Path, process: subprocess.Popen[str]) -> dict[str, str]:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if path.is_file():
            return json.loads(path.read_text(encoding="utf-8"))
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr is not None else ""
            raise AssertionError(f"fixture server exited early: {stderr}")
        time.sleep(0.02)
    raise AssertionError("fixture server did not publish readiness")


def _channel(root: Path, payloads: dict[str, bytes]) -> Path:
    """A manifest declaring each payload it ships, and the files beside it.

    The legacy schema, because it is the smaller of the two the enumerator
    accepts and this is a test of verification rather than of parsing -- the
    release-graph schema is covered where that parsing lives.
    """
    version = "2026.0820.1"
    arch = "x86_64"
    release = root / "assets" / "releases" / version
    release.mkdir(parents=True, exist_ok=True)
    files = {}
    for name, payload in payloads.items():
        (release / f"{arch}-{name}").write_bytes(payload)
        files[name] = {
            "hash": blake3.blake3(payload).hexdigest(),
            "size": len(payload),
        }
    manifest = {
        "channel": "local",
        "asset_base": "/assets/releases",
        "assets": {"current": version, "releases": {version: {"arches": {arch: files}}}},
    }
    path = root / "manifest.json"
    path.write_text(json.dumps(manifest), encoding="utf-8")
    return path


@pytest.fixture
def channel(tmp_path: Path):
    """A throwaway channel on loopback, and the manifest it serves."""
    root = tmp_path / "channel"
    root.mkdir()
    manifest = _channel(root, {"rootfs.erofs": b"rootfs-bytes", "initrd.img": b"initrd"})
    ready = tmp_path / "ready.json"
    process = subprocess.Popen(
        [sys.executable, str(SERVER), "--root", str(root), "--ready-file", str(ready)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        state = _wait_for_ready(ready, process)
        yield root, manifest, f"{state['base_url']}/manifest.json"
    finally:
        process.terminate()
        process.wait(timeout=10)
        # Closed explicitly: an unclosed pipe surfaces as a teardown error on
        # every test in the module, which reads like four failures rather than
        # one fixture leaking two file objects.
        for pipe in (process.stdout, process.stderr):
            if pipe is not None:
                pipe.close()


def _verify(manifest: Path, url: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(VERIFY), "--manifest-path", str(manifest), "--manifest-url", url],
        check=False,
        capture_output=True,
        text=True,
    )


def test_a_channel_that_serves_what_it_declares_verifies(channel) -> None:
    _root, manifest, url = channel

    result = _verify(manifest, url)

    assert result.returncode == 0, result.stdout + result.stderr
    assert "downloads and matches its digest" in result.stdout


def test_a_row_whose_bytes_are_gone_is_named(channel) -> None:
    """The failure the live stable channel had for a month."""
    root, manifest, url = channel
    gone = next(root.rglob("x86_64-rootfs.erofs"))
    gone.unlink()

    result = _verify(manifest, url)

    assert result.returncode == 1
    assert "not reachable (HTTP 404)" in result.stdout
    assert "rootfs.erofs" in result.stdout


def test_a_row_of_the_wrong_length_is_named(channel) -> None:
    root, manifest, url = channel
    grown = next(root.rglob("x86_64-initrd.img"))
    grown.write_bytes(b"initrd" + b"extra")

    result = _verify(manifest, url)

    assert result.returncode == 1
    assert "the manifest declares" in result.stdout
    assert "initrd.img" in result.stdout


def test_a_row_whose_content_changed_is_named(channel) -> None:
    """Same length, different bytes: only the digest can see it."""
    root, manifest, url = channel
    swapped = next(root.rglob("x86_64-rootfs.erofs"))
    swapped.write_bytes(b"ROOTFS-BYTES")

    result = _verify(manifest, url)

    assert result.returncode == 1
    assert "hashes to" in result.stdout
    assert "rootfs.erofs" in result.stdout
