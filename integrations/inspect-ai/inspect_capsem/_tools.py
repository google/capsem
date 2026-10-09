"""Inspect sandbox tools binary resolution, baking, and file transfer helpers."""

from __future__ import annotations

import logging
import os
import platform
import re
import shlex
from pathlib import Path

from inspect_capsem._controller import CapsemController

logger = logging.getLogger(__name__)

INSPECT_SANDBOX_TOOLS_GUEST_DIR = "/var/tmp/sandbox-services"
INSPECT_SANDBOX_TOOLS_GUEST_PATH = f"{INSPECT_SANDBOX_TOOLS_GUEST_DIR}/inspect-sandbox-tools"
# Mirrors `SANDBOX_TOOLS_DIR` (`inspect_ai/util/_sandbox/_cli.py:31`) in `inspect_ai`'s
# `inspect-sandbox-tools` onedir wrapper so pre-extracting v29+ `.tar.gz` archives avoids
# per-invocation extraction overhead. If a future `inspect_ai` build changes this directory
# hash, the binary falls back to self-extracting on first invocation.
INSPECT_SANDBOX_TOOLS_ONEDIR_PATH = "/var/tmp/.da7be258e003d428"


def resolve_inspect_sandbox_tools_host_binary(
    override_path: str | Path | None = None,
) -> Path | None:
    """Locate the host `inspect-sandbox-tools` binary matching the guest architecture."""
    if override_path is not None:
        candidate = Path(override_path)
        return candidate if candidate.is_file() else None
    env_override = os.environ.get("INSPECT_CAPSEM_SANDBOX_TOOLS_PATH")
    if env_override and Path(env_override).is_file():
        return Path(env_override)
    try:
        import inspect_ai

        arch = "arm64" if platform.machine().lower() in ("aarch64", "arm64") else "amd64"
        binaries_dir = Path(inspect_ai.__file__).resolve().parent / "binaries"
        exact = binaries_dir / f"inspect-sandbox-tools-{arch}-linux"
        if exact.is_file():
            return exact
        all_candidates = [
            p
            for p in binaries_dir.glob(f"inspect-sandbox-tools-{arch}*")
            if p.is_file() and not p.name.endswith(".tar")
        ]
        candidates = [p for p in all_candidates if "-musl-" not in p.name] or all_candidates
        if candidates:

            def _version_key(p: Path) -> tuple[int, str]:
                match = re.search(r"-v(\d+)$", p.name)
                return (int(match.group(1)) if match else 0, p.name)

            candidates.sort(key=_version_key, reverse=True)
            return candidates[0]
    except Exception:
        logger.debug("Failed locating inspect-sandbox-tools in inspect_ai", exc_info=True)
    return None


def _onedir_extract_command() -> str:
    onedir_q = shlex.quote(INSPECT_SANDBOX_TOOLS_ONEDIR_PATH)
    guest_path_q = shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_PATH)
    return (
        f"mkdir -p {onedir_q} && "
        f"tar -xzf {guest_path_q} -C {onedir_q} && "
        f"(chown -R root:root {onedir_q} 2>/dev/null || true) && "
        f"chmod -R 700 {onedir_q}"
    )


async def bake_sandbox_tools_into_controller(
    controller: CapsemController,
    vm_id: str,
    *,
    host_binary_path: str | Path | None = None,
) -> bool:
    """Install `inspect-sandbox-tools` into `/var/tmp/sandbox-services/` (0700 root:root)."""
    binary = resolve_inspect_sandbox_tools_host_binary(host_binary_path)
    if binary is None or not binary.is_file():
        logger.warning(
            "Could not locate host inspect-sandbox-tools binary to bake into VM %s",
            vm_id,
        )
        return False

    if (
        await controller.exec_in_vm(
            vm_id, f"test -x {shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_PATH)}", timeout=60
        )
    ).exit_code == 0:
        return True

    raw = binary.read_bytes()
    guest_path_q = shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_PATH)
    extract_suffix = f" && {_onedir_extract_command()}" if raw[:2] == b"\x1f\x8b" else ""
    await controller.upload_to_vm(vm_id, INSPECT_SANDBOX_TOOLS_GUEST_PATH, raw)
    res = await controller.exec_in_vm(
        vm_id, f"chmod 700 {guest_path_q}{extract_suffix}", timeout=120
    )
    if res.exit_code != 0:
        logger.warning("Failed baking inspect-sandbox-tools into VM %s: %s", vm_id, res.stderr)
        return False
    return True
