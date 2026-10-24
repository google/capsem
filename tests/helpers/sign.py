"""Role-owned fixture signing and read-only frozen artifact verification."""

import os
import plistlib
import subprocess
from pathlib import Path

from capsem_builder.gate.config import load
from capsem_builder.release.tools.test_binary_provenance import (
    COHORT_VARIABLE,
    verify_frozen_binary,
)

from .constants import host_bin_root

PROJECT_ROOT = Path(__file__).parent.parent.parent

IS_MACOS = os.uname().sysname == "Darwin"


def _selected_role(binary_path: Path) -> bool:
    return (
        binary_path.name in load(PROJECT_ROOT).signing.built
        and binary_path.parent.resolve() == host_bin_root().resolve()
    )


def _needs_entitlements(binary_path: Path) -> bool:
    return _selected_role(binary_path) and binary_path.name in {
        Path(path).name for path in load(PROJECT_ROOT).signing.binaries
    }


def sign_binary(binary_path: Path) -> None:
    """Verify immutable inputs; prepare only ordinary developer executables.

    Role-specific entitlements belong to the gate's signing policy. Frozen or
    release bytes must never be repaired in place, including lock-file writes.
    """
    cohort = os.environ.get(COHORT_VARIABLE, "").strip()
    release = os.environ.get("CAPSEM_RELEASE_INPUT_DIR", "").strip()
    immutable = _selected_role(binary_path) and bool(cohort or release)
    if immutable:
        if cohort and release:
            raise RuntimeError("frozen provenance cannot be mixed with release inputs")
        if cohort:
            verify_frozen_binary(binary_path, Path(cohort), PROJECT_ROOT)
        if not binary_path.is_file() or not os.access(binary_path, os.X_OK):
            raise RuntimeError(f"selected binary is missing or not executable: {binary_path}")
        if not verify_signed(binary_path):
            raise RuntimeError(f"selected binary signature is invalid: {binary_path}")
        return
    if not IS_MACOS:
        return

    if not binary_path.exists():
        raise FileNotFoundError(f"Binary not found: {binary_path}")

    entitlements = load(PROJECT_ROOT).path(load(PROJECT_ROOT).signing.entitlements)
    required = _needs_entitlements(binary_path)
    if required and not entitlements.exists():
        raise FileNotFoundError(f"Entitlements not found: {entitlements}")

    import fcntl
    lock_path = binary_path.with_suffix(".sign.lock")
    with open(lock_path, "w") as lock_fd:
        fcntl.flock(lock_fd, fcntl.LOCK_EX)
        # Skip if already validly signed
        if verify_signed(binary_path):
            return
        command = ["codesign", "--sign", "-", "--force"]
        if required:
            command.extend(["--entitlements", str(entitlements)])
        result = subprocess.run(
            [*command, str(binary_path)],
            capture_output=True, text=True, check=False,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"Failed to sign {binary_path.name}: {result.stderr.strip()}"
            )


def verify_signed(binary_path: Path) -> bool:
    """Check a strict signature and only this role's configured entitlements."""
    if not IS_MACOS:
        return True

    # 1. Basic signature check
    result = subprocess.run(
        ["codesign", "--verify", "--strict", str(binary_path)],
        capture_output=True, text=True, check=False,
    )
    if result.returncode != 0:
        return False
    if not _needs_entitlements(binary_path):
        return True

    # 2. Entitlement check (codesign -d --entitlements -)
    # This ensures the virtualization entitlement is actually baked in.
    result = subprocess.run(
        ["codesign", "-d", "--entitlements", "-", str(binary_path)],
        capture_output=True, text=True, check=False,
    )
    if result.returncode != 0:
        return False
    try:
        actual = plistlib.loads(result.stdout.encode())
        settings = load(PROJECT_ROOT)
        required = plistlib.loads(settings.path(settings.signing.entitlements).read_bytes())
    except (ValueError, plistlib.InvalidFileException):
        return False
    return isinstance(actual, dict) and all(
        type(actual.get(key)) is type(value) and actual[key] == value
        for key, value in required.items()
    )


def ensure_all_signed() -> None:
    """Sign all daemon binaries. Call once before any VM test."""
    from . import service as svc_mod
    for binary in [svc_mod.SERVICE_BINARY, svc_mod.PROCESS_BINARY]:
        if binary.exists():
            sign_binary(binary)
