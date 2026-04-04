"""Shared fixtures for build chain E2E tests.

Validates cargo build -> codesign -> pack-initrd -> manifest -> boot.
"""

import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"
TARGET_DIR = PROJECT_ROOT / "target" / "debug"
ENTITLEMENTS = PROJECT_ROOT / "entitlements.plist"

IS_MACOS = os.uname().sysname == "Darwin"

DAEMON_CRATES = ["capsem-service", "capsem-process", "capsem", "capsem-ui", "capsem-mcp"]
DAEMON_BINARIES = {
    "capsem-service": TARGET_DIR / "capsem-service",
    "capsem-process": TARGET_DIR / "capsem-process",
    "capsem": TARGET_DIR / "capsem",
    "capsem-ui": TARGET_DIR / "capsem-ui",
    "capsem-mcp": TARGET_DIR / "capsem-mcp",
}

pytestmark = pytest.mark.build_chain


def host_arch():
    return "arm64" if os.uname().machine == "arm64" else "x86_64"


@pytest.fixture(scope="session")
def built_binaries():
    """Build all daemon crates once for the session."""
    result = subprocess.run(
        ["cargo", "build"] + [arg for c in DAEMON_CRATES for arg in ["-p", c]],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=300,
    )
    assert result.returncode == 0, f"cargo build failed:\n{result.stderr}"
    return DAEMON_BINARIES


@pytest.fixture(scope="session")
def signed_binaries(built_binaries):
    """Sign all daemon binaries (macOS only)."""
    if not IS_MACOS:
        return built_binaries

    for name, path in built_binaries.items():
        if not path.exists():
            continue
        result = subprocess.run(
            [
                "codesign", "--sign", "-",
                "--entitlements", str(ENTITLEMENTS),
                "--force",
                str(path),
            ],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"Failed to sign {name}: {result.stderr}"
    return built_binaries
