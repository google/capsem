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

    No-op on Linux. Uses a file lock to prevent races when multiple
    test processes sign concurrently.
    """
    if not IS_MACOS:
        return

    if not binary_path.exists():
        raise FileNotFoundError(f"Binary not found: {binary_path}")

    if not ENTITLEMENTS.exists():
        raise FileNotFoundError(f"Entitlements not found: {ENTITLEMENTS}")

    import fcntl
    lock_path = binary_path.with_suffix(".sign.lock")
    with open(lock_path, "w") as lock_fd:
        fcntl.flock(lock_fd, fcntl.LOCK_EX)
        # Skip if already validly signed
        if verify_signed(binary_path):
            return
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
    """Check if a binary is validly signed and has required entitlements."""
    if not IS_MACOS:
        return True

    # 1. Basic signature check
    result = subprocess.run(
        ["codesign", "--verify", str(binary_path)],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return False

    # 2. Entitlement check (codesign -d --entitlements -)
    # This ensures the virtualization entitlement is actually baked in.
    result = subprocess.run(
        ["codesign", "-d", "--entitlements", "-", str(binary_path)],
        capture_output=True, text=True,
    )
    return "com.apple.security.virtualization" in result.stdout


def ensure_all_signed() -> None:
    """Sign all daemon binaries. Call once before any VM test."""
    from . import service as svc_mod
    for binary in [svc_mod.SERVICE_BINARY, svc_mod.PROCESS_BINARY]:
        if binary.exists():
            sign_binary(binary)
