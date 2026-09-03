#!/usr/bin/env python3
"""Remove stale Capsem build artifacts, test fixtures, and orphan UDS sockets.

Replaces the bash body of `just _clean-stale`. The bash version called
`lsof -tU` once per socket, which on macOS costs ~200 ms each and made the
loop take ~6 minutes once /tmp/capsem/ accumulated ~1700 sockets. This
probes liveness via socket.connect() instead (~4 us per socket).
"""

from __future__ import annotations

import argparse
import errno
import fnmatch
import json
import os
import socket
import sys
import time
from dataclasses import asdict
from pathlib import Path

from capsem_builder.cache.config import load_paths

from .cleanup_common import (
    StageResult,
    allocated_size_bytes,
    human_bytes,
)
from .cleanup_common import remove_path as _rm
from .cleanup_tmp import (
    clean_tmp_fixtures,
    clean_tmp_fixtures_to_budget,
    tmp_fixture_roots,
)

SOCKET_CONNECT_TIMEOUT_S = 0.05
TARGET_TRANSIENT_MAX_AGE_S = 6 * 60 * 60
TARGET_TRANSIENT_GLOBS = (
    "asset-release",
    "asset-release-delta",
    "generated-settings-*",
    "local-release-glowup*",
    "release-channel-local*",
    "release-contract-artifacts*",
    "pkg-expand-test*",
    "*-proof-*",
    "focused-*-rootfs-*",
    "ironbank-assets-debug*",
    "ironbank-assets-sequential*",
    "s??-???-channel",
    "s??-???-release-dist",
)


def clean_rootfs_scratch(root: Path, dry_run: bool, verbose: bool) -> StageResult:
    """Stage A: remove `*/debug/rootfs.*`, `*/release/rootfs.*`, and `_up_` dirs under cache/target/."""
    start = time.monotonic()
    target = root / "cache" / "target" / "cargo"
    if not target.is_dir():
        return StageResult("rootfs", 0, time.monotonic() - start)

    removed = 0
    seen: set[Path] = set()

    for path in target.rglob("rootfs.*"):
        if path in seen or not path.is_dir():
            continue
        if path.parent.name not in {"debug", "release"}:
            continue
        seen.add(path)
        if verbose:
            print(f"  rm {path}")
        if _rm(path, dry_run):
            removed += 1

    for path in target.rglob("_up_"):
        if path in seen or not path.is_dir():
            continue
        seen.add(path)
        if verbose:
            print(f"  rm {path}")
        if _rm(path, dry_run):
            removed += 1

    return StageResult("rootfs", removed, time.monotonic() - start)


def _socket_is_alive(path: Path) -> bool:
    """True if the UDS at `path` has a live listener. False if ECONNREFUSED.

    Raises on unexpected errors so the caller can keep the socket rather than
    delete a file we failed to probe.
    """
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(SOCKET_CONNECT_TIMEOUT_S)
    try:
        s.connect(str(path))
        return True
    except ConnectionRefusedError:
        return False
    except (TimeoutError, BlockingIOError):
        return True
    except OSError as e:
        if e.errno in (errno.EWOULDBLOCK, errno.EAGAIN, errno.EINPROGRESS):
            return True
        raise
    finally:
        s.close()


def clean_orphan_sockets(sockets_dir: Path, dry_run: bool, verbose: bool) -> StageResult:
    """Stage B: remove .sock files with no listener and their .ready companions."""
    start = time.monotonic()
    if not sockets_dir.is_dir():
        return StageResult("sockets", 0, time.monotonic() - start)

    removed = 0
    errors = 0
    try:
        with os.scandir(sockets_dir) as it:
            entries = list(it)
    except OSError:
        return StageResult("sockets", 0, time.monotonic() - start)

    for entry in entries:
        if not entry.name.endswith(".sock"):
            continue
        sock_path = Path(entry.path)
        try:
            alive = _socket_is_alive(sock_path)
        except OSError:
            errors += 1
            continue
        if alive:
            continue
        if verbose:
            print(f"  rm {sock_path}")
        if _rm(sock_path, dry_run):
            removed += 1
        ready_path = sock_path.with_suffix(".ready")
        if ready_path.exists():
            if verbose:
                print(f"  rm {ready_path}")
            _rm(ready_path, dry_run)

    detail = f"{errors} probe error(s)" if errors else ""
    return StageResult("sockets", removed, time.monotonic() - start, detail)


def clean_target_transients(root: Path, dry_run: bool, verbose: bool) -> StageResult:
    """Remove old reproducible proof/debug staging without touching hot caches."""
    start = time.monotonic()
    target = root / "cache" / "target"
    if not target.is_dir():
        return StageResult("target-tmp", 0, time.monotonic() - start, "cache/target/ absent")

    cutoff = time.time() - TARGET_TRANSIENT_MAX_AGE_S
    candidates: list[Path] = []

    scratch = target / "tmp"
    if scratch.is_dir():
        try:
            with os.scandir(scratch) as entries:
                candidates.extend(Path(entry.path) for entry in entries)
        except OSError:
            pass

    try:
        with os.scandir(target) as entries:
            for entry in entries:
                if entry.name == "tmp" or not entry.is_dir(follow_symlinks=False):
                    continue
                if any(fnmatch.fnmatch(entry.name, pattern) for pattern in TARGET_TRANSIENT_GLOBS):
                    candidates.append(Path(entry.path))
    except OSError:
        pass

    removed = 0
    for path in candidates:
        try:
            if path.stat().st_mtime >= cutoff:
                continue
        except OSError:
            continue
        if verbose:
            print(f"  rm {path} (old reproducible staging)")
        if _rm(path, dry_run):
            removed += 1

    return StageResult(
        "target-tmp",
        removed,
        time.monotonic() - start,
        f"threshold={TARGET_TRANSIENT_MAX_AGE_S // 3600:g}h",
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=os.getcwd(), help="Project root (default: cwd)")
    parser.add_argument("--tmp-dir", default=os.environ.get("TMPDIR", "/tmp"))
    parser.add_argument("--sockets-dir", default="/tmp/capsem")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--verbose", "-v", action="store_true")
    parser.add_argument("--skip-target-transients", action="store_true")
    parser.add_argument("--skip-sockets", action="store_true")
    parser.add_argument("--skip-rootfs", action="store_true")
    parser.add_argument("--skip-tmp", action="store_true")
    parser.add_argument(
        "--report",
        help="JSONL cleanup ledger (default: policy-owned cache state)",
    )
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    tmp_dir = Path(args.tmp_dir)
    sockets_dir = Path(args.sockets_dir)

    print("=== Pruning stale build artifacts ===")
    total_start = time.monotonic()
    target_path = root / "cache" / "target"
    target_before = allocated_size_bytes(target_path) or 0
    results: list[StageResult] = []

    if not args.skip_rootfs:
        results.append(clean_rootfs_scratch(root, args.dry_run, args.verbose))
    if not args.skip_sockets:
        results.append(clean_orphan_sockets(sockets_dir, args.dry_run, args.verbose))
    if not args.skip_tmp:
        for root_dir in tmp_fixture_roots(tmp_dir):
            results.append(clean_tmp_fixtures(root_dir, args.dry_run, args.verbose))
            results.append(clean_tmp_fixtures_to_budget(root_dir, args.dry_run, args.verbose))
    if not args.skip_target_transients:
        results.append(clean_target_transients(root, args.dry_run, args.verbose))

    for r in results:
        suffix = f" [{r.detail}]" if r.detail else ""
        byte_delta = ""
        if r.bytes_before or r.bytes_after:
            byte_delta = (
                f" bytes={human_bytes(r.bytes_before)}->{human_bytes(r.bytes_after)}"
                f" reclaimed={human_bytes(r.bytes_reclaimed)}"
            )
        print(
            f"  {r.name:8s} removed={r.removed:<6d} "
            f"{r.elapsed_s * 1000:7.0f} ms{byte_delta}{suffix}"
        )

    target_after = allocated_size_bytes(target_path) or 0
    if target_before or target_after:
        print(
            f"  cache/target/: {human_bytes(target_before)} -> {human_bytes(target_after)} "
            f"(reclaimed {human_bytes(max(0, target_before - target_after))})"
        )

    total = time.monotonic() - total_start
    ledger = (
        Path(args.report)
        if args.report
        else load_paths(root).stage("state") / "host-cleanup.jsonl"
    )
    ledger.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema": "capsem.host_cleanup.v1",
        "timestamp": time.time(),
        "root": str(root),
        "dry_run": args.dry_run,
        "target": {
            "before_bytes": target_before,
            "after_bytes": target_after,
            "reclaimed_bytes": max(0, target_before - target_after),
        },
        "stages": [
            {**asdict(result), "bytes_reclaimed": result.bytes_reclaimed} for result in results
        ],
        "elapsed_s": total,
    }
    with ledger.open("a") as stream:
        stream.write(json.dumps(payload, sort_keys=True) + "\n")
    print(f"  ledger: {ledger}")
    print(f"=== Done in {total:.1f}s ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
