"""Guest binary permissions: chmod 555, read-only filesystem."""

import os
import subprocess
import stat

import pytest

from pathlib import Path

from capsem.builder.docker import GUEST_BINARIES

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"

pytestmark = pytest.mark.security


def _host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


def test_agent_binaries_555():
    """Guest binaries in target/linux-agent/ should be chmod 555."""
    arch = _host_arch()
    agent_dir = PROJECT_ROOT / "target" / "linux-agent" / arch
    if not agent_dir.exists():
        pytest.skip(f"Agent dir not found: {agent_dir}")

    for name in GUEST_BINARIES:
        binary = agent_dir / name
        assert binary.exists(), f"{name} missing from {agent_dir}"
        mode = oct(binary.stat().st_mode & 0o777)
        assert mode == "0o555", f"{name} should be 555, got {mode}"


def test_initrd_binaries_555():
    """After _pack-initrd, binaries inside the initrd retain 555 permissions.

    Extracts the initrd to a temp dir and checks permissions.
    """
    arch = _host_arch()
    initrd = ASSETS_DIR / arch / "initrd.img"
    if not initrd.exists():
        pytest.skip("No initrd.img")

    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        # Extract initrd (gzip + cpio)
        result = subprocess.run(
            f"gunzip -c {initrd} | cpio -id 2>/dev/null",
            shell=True, cwd=tmp, capture_output=True,
        )
        if result.returncode != 0:
            pytest.skip(f"Could not extract initrd: {result.stderr.decode()}")

        for name in GUEST_BINARIES:
            # Binaries might be at root or in usr/bin
            candidates = list(Path(tmp).rglob(name))
            assert candidates, f"{name} missing from initrd"
            binary = candidates[0]
            mode = oct(binary.stat().st_mode & 0o777)
            assert mode == "0o555", f"{name} in initrd should be 555, got {mode}"
