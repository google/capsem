"""Resolve host executables for local and release functional tests.

Local qualification owns source builds. Release qualification owns no hidden
binary build: it must execute the package bytes staged from the selected
manifest, even when checkout mtimes are newer than immutable package mtimes.
Local binaries live in a shared Cargo target, so content receipts distinguish
the current source from newer output left by another worktree.
"""

from __future__ import annotations

import fcntl
import json
import os
import subprocess
import sys
from collections.abc import Callable, Iterable, Mapping, Sequence
from pathlib import Path

from capsem_builder import crypto

Runner = Callable[..., subprocess.CompletedProcess[str]]
RECEIPT_SCHEMA = "capsem.local-test-binary.v1"


def _digest_file(path: Path) -> str:
    return crypto.digest_file(path)


def _source_digest(
    sources: Sequence[Path],
    *,
    project_root: Path,
    build_command: Sequence[str],
    env: Mapping[str, str],
) -> str:
    digest = crypto.blake3()
    identity = {
        "build_command": list(build_command),
        "qualified_source": env.get("CAPSEM_GATE_QUALIFIED_SOURCE_COMMIT", ""),
    }
    digest.update(json.dumps(identity, sort_keys=True, separators=(",", ":")).encode())
    root = project_root.resolve()
    for source in sorted(sources, key=lambda path: str(path)):
        resolved = source.resolve()
        try:
            label = resolved.relative_to(root).as_posix()
        except ValueError:
            label = resolved.as_posix()
        digest.update(b"\0")
        digest.update(label.encode())
        digest.update(b"\0")
        with resolved.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def _receipt_path(binary: Path) -> Path:
    return binary.with_name(f".{binary.name}.capsem-source.json")


def _receipt_matches(receipt: Path, binary: Path, source_digest: str) -> bool:
    if not binary.is_file() or not os.access(binary, os.X_OK):
        return False
    try:
        payload = json.loads(receipt.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    return (
        payload.get("schema") == RECEIPT_SCHEMA
        and payload.get("source_digest") == source_digest
        and payload.get("binary_digest") == _digest_file(binary)
    )


def _write_receipt(receipt: Path, binary: Path, source_digest: str) -> None:
    payload = {
        "schema": RECEIPT_SCHEMA,
        "source_digest": source_digest,
        "binary_digest": _digest_file(binary),
    }
    temporary = receipt.with_name(f".{receipt.name}.{os.getpid()}.tmp")
    try:
        temporary.write_text(
            json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, receipt)
    finally:
        temporary.unlink(missing_ok=True)


def ensure_host_test_binary(
    binary: Path,
    *,
    source_paths: Iterable[Path],
    build_command: Sequence[str],
    project_root: Path,
    env: Mapping[str, str] = os.environ,
    runner: Runner = subprocess.run,
    timeout: int = 120,
) -> None:
    """Make one test executable available without crossing release ownership."""
    release_inputs = env.get("CAPSEM_RELEASE_INPUT_DIR", "").strip()
    if release_inputs:
        if not binary.is_file():
            raise FileNotFoundError(
                "manifest-selected package lacks required functional-test binary: "
                f"{binary}"
            )
        if not os.access(binary, os.X_OK):
            raise PermissionError(
                "manifest-selected package binary is not executable: "
                f"{binary}"
            )
        return

    sources = tuple(Path(path) for path in source_paths)
    source_digest = _source_digest(
        sources,
        project_root=project_root,
        build_command=build_command,
        env=env,
    )
    receipt = _receipt_path(binary)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    lock_path = receipt.with_name(f"{receipt.name}.lock")
    with lock_path.open("a+", encoding="utf-8") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        if _receipt_matches(receipt, binary, source_digest):
            return

        # The wrapper's child timeout starts after its kernel-lease wait. An outer
        # runner timeout would spend that wait and abandon the owning wrapper.
        runner(
            (sys.executable, str(project_root / "build_system/scripts/ci/run-bounded-command.py"),
             "--timeout-seconds", str(timeout), "--", *build_command),
            cwd=project_root,
            check=True,
        )
        if not binary.is_file():
            raise FileNotFoundError(f"local build did not create required binary: {binary}")
        if not os.access(binary, os.X_OK):
            raise PermissionError(f"local build created a non-executable binary: {binary}")
        _write_receipt(receipt, binary, source_digest)
