"""Real-root regression for Debian maintainer scripts entering a user's home."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

PACKAGING = Path(__file__).resolve().parents[2] / "packaging"
IMAGE = "ubuntu:24.04"


def _require_image() -> None:
    if shutil.which("docker") is None:
        pytest.skip("Docker is not available")
    if (
        subprocess.run(
            ["docker", "image", "inspect", IMAGE],
            capture_output=True,
            check=False,
            timeout=10,
        ).returncode
        != 0
    ):
        pytest.skip(f"{IMAGE} is not locally available")


def _run_container(script: str, setup: str, *, user: str = "captest") -> str:
    _require_image()
    command = f"""
set -eu
useradd -m -u 1999 {user}
mkdir -p /home/{user}/.capsem/logs
chown -R {user}:{user} /home/{user}/.capsem
printf KEEP > /root/capsem-canary
{setup}
SUDO_USER={user} USER=root bash /src/linux/{script} >/tmp/install.out 2>&1 || true
printf 'CANARY:%s\\n' "$(cat /root/capsem-canary)"
cat /tmp/install.out
"""
    result = subprocess.run(
        [
            "docker",
            "run",
            "--pull=never",
            "--rm",
            "-v",
            f"{PACKAGING}:/src:ro",
            IMAGE,
            "bash",
            "-c",
            command,
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=45,
    )
    assert result.returncode == 0, result.stderr
    return result.stdout


@pytest.mark.parametrize("script", ["deb-preinst.sh", "deb-postinst.sh"])
def test_user_log_symlink_cannot_redirect_root_write(script: str) -> None:
    output = _run_container(
        script,
        "ln -s /root/capsem-canary /home/captest/.capsem/logs/install.log",
    )
    assert "CANARY:KEEP\n" in output


def test_manifest_destination_symlink_cannot_redirect_root_write() -> None:
    output = _run_container(
        "deb-postinst.sh",
        """
mkdir -p /usr/share/capsem/assets /home/captest/.capsem/assets
printf '{"manifest_url":"file:///tmp/manifest.json"}\\n' \
    > /usr/share/capsem/assets/manifest-metadata.json
chown -R captest:captest /home/captest/.capsem/assets
ln -s /root/capsem-canary \
    /home/captest/.capsem/assets/manifest-metadata.json
""",
    )
    assert "CANARY:KEEP\n" in output


@pytest.mark.parametrize("script", ["deb-preinst.sh", "deb-postinst.sh"])
def test_no_implicit_uid_1000_home_selection(script: str) -> None:
    _require_image()
    command = f"""
set -eu
uid_home=$(getent passwd 1000 | cut -d: -f6)
mkdir -p "$uid_home/.capsem/logs"
printf KEEP > /root/capsem-canary
ln -s /root/capsem-canary "$uid_home/.capsem/logs/install.log"
if env -u SUDO_USER USER=root bash /src/linux/{script} >/tmp/install.out 2>&1; then
    exit 9
fi
printf 'CANARY:%s\\n' "$(cat /root/capsem-canary)"
cat /tmp/install.out
"""
    result = subprocess.run(
        [
            "docker",
            "run",
            "--pull=never",
            "--rm",
            "-v",
            f"{PACKAGING}:/src:ro",
            IMAGE,
            "bash",
            "-c",
            command,
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=45,
    )
    assert result.returncode == 0, result.stderr
    assert "CANARY:KEEP\n" in result.stdout
    assert "no installing user supplied" in result.stdout
