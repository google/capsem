"""Codesigning helpers for test fixtures.

Signs binaries with the virtualization entitlement before spawning.
Skips on Linux (KVM doesn't need entitlements).
"""

import os
import subprocess
import sys

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ENTITLEMENTS = PROJECT_ROOT / "entitlements.plist"

IS_MACOS = os.uname().sysname == "Darwin"


def sign_binary(binary_path: Path) -> None:
    """Sign a binary with the virtualization entitlement.

    No-op on Linux. Raises RuntimeError on macOS if signing fails.
    """
    if not IS_MACOS:
        return

    if not binary_path.exists():
        raise FileNotFoundError(f"Binary not found: {binary_path}")

    if not ENTITLEMENTS.exists():
        raise FileNotFoundError(f"Entitlements not found: {ENTITLEMENTS}")

    result = subprocess.run(
        [
            "codesign", "--sign", "-",
            "--entitlements", str(ENTITLEMENTS),
            "--force",
            str(binary_path),
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"Failed to sign {binary_path.name}: {result.stderr.strip()}"
        )


def verify_signed(binary_path: Path) -> bool:
    """Check if a binary is validly signed. Returns False on Linux."""
    if not IS_MACOS:
        return True

    result = subprocess.run(
        ["codesign", "--verify", "--verbose", str(binary_path)],
        capture_output=True, text=True,
    )
    return result.returncode == 0


def ensure_all_signed() -> None:
    """Sign all daemon binaries. Call once before any VM test."""
    from . import service as svc_mod
    for binary in [svc_mod.SERVICE_BINARY, svc_mod.PROCESS_BINARY]:
        if binary.exists():
            sign_binary(binary)
