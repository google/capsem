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
import os
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path


SOCKET_CONNECT_TIMEOUT_S = 0.05
TMP_DIR_MAX_AGE_S = 60 * 60  # 1 hour
CARGO_AGGRESSIVE_DAYS = 2
CARGO_MODERATE_DAYS = 3
CARGO_PROFILES = ("target/debug", "target/release", "target/llvm-cov-target/debug")
CARGO_DEPS_EXTS = (".o", ".rlib", ".rmeta", ".d")
CARGO_KIND_DIRS = ("build", ".fingerprint", "incremental")
# Per-kind size caps enforced AFTER the mtime-based prune. The age prune
# alone is insufficient: an active dev session touches every dep/incremental
# dir on every build, so the 2-3 day age threshold never fires and target/
# grows unbounded (72 GB observed; 23 GB alone in incremental/). Budgets
# picked empirically to retain a useful warm cache without letting the
# footprint run away.
CARGO_KIND_BUDGETS_GB = {
    "deps": 12.0,
    "incremental": 3.0,
    "build": 1.0,
    ".fingerprint": 0.5,
}
TMP_DIR_PREFIXES = ("capsem-test-", "capsem-e2e-", "capsem-gw-", "capsem-install-")


@dataclass
class StageResult:
    name: str
    removed: int
    elapsed_s: float
    detail: str = ""


def _rm(path: Path, dry_run: bool) -> bool:
    if dry_run:
        return True
    try:
        if path.is_symlink() or path.is_file():
            path.unlink(missing_ok=True)
        elif path.is_dir():
            shutil.rmtree(path, ignore_errors=True)
        else:
            try:
                path.unlink()
            except FileNotFoundError:
                return False
        return True
    except OSError:
        return False


def clean_rootfs_scratch(root: Path, dry_run: bool, verbose: bool) -> StageResult:
    """Stage A: remove `*/debug/rootfs.*`, `*/release/rootfs.*`, and `_up_` dirs under target/."""
    start = time.monotonic()
    target = root / "target"
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
    except (BlockingIOError, socket.timeout):
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
        entries = list(os.scandir(sockets_dir))
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


def clean_tmp_fixtures(tmp_dir: Path, dry_run: bool, verbose: bool) -> StageResult:
    """Stage C: remove stale capsem-* test fixture dirs older than 1 hour."""
    start = time.monotonic()
    if not tmp_dir.is_dir():
        return StageResult("tmp", 0, time.monotonic() - start)

    cutoff = time.time() - TMP_DIR_MAX_AGE_S
    removed = 0
    try:
        entries = list(os.scandir(tmp_dir))
    except OSError:
        return StageResult("tmp", 0, time.monotonic() - start)

    for entry in entries:
        if not entry.is_dir(follow_symlinks=False):
            continue
        if not any(entry.name.startswith(p) for p in TMP_DIR_PREFIXES):
            continue
        try:
            mtime = entry.stat(follow_symlinks=False).st_mtime
        except OSError:
            continue
        if mtime >= cutoff:
            continue
        path = Path(entry.path)
        if verbose:
            print(f"  rm {path}")
        if _rm(path, dry_run):
            removed += 1

    return StageResult("tmp", removed, time.monotonic() - start)


def _entry_size_bytes(entry: os.DirEntry) -> int:
    """Size of entry: the file's own bytes for a file, total bytes for a dir."""
    try:
        if entry.is_symlink():
            return 0
        if entry.is_file(follow_symlinks=False):
            return entry.stat(follow_symlinks=False).st_size
        if entry.is_dir(follow_symlinks=False):
            total = 0
            for root_dir, _dirs, files in os.walk(entry.path):
                for name in files:
                    try:
                        total += os.lstat(os.path.join(root_dir, name)).st_size
                    except OSError:
                        continue
            return total
    except OSError:
        return 0
    return 0


def _prune_to_size_budget(
    parent: Path,
    budget_bytes: int,
    entry_filter,
    dry_run: bool,
    verbose: bool,
) -> int:
    """Delete oldest entries under `parent` until total size is <= budget_bytes.

    `entry_filter(entry)` must return True for entries eligible for pruning;
    ineligible entries are ignored (their size is NOT counted toward budget
    either, so this keeps the cap honest for what it's allowed to touch).

    Strategy: size every eligible entry, sort oldest-mtime first, delete in
    order until the running total drops to/under the budget. Newest entries
    are preserved so a warm cache survives.

    Returns the number of entries removed.
    """
    if not parent.is_dir():
        return 0
    try:
        scored: list[tuple[float, int, Path]] = []
        total = 0
        for entry in os.scandir(parent):
            if not entry_filter(entry):
                continue
            try:
                mtime = entry.stat(follow_symlinks=False).st_mtime
            except OSError:
                continue
            size = _entry_size_bytes(entry)
            scored.append((mtime, size, Path(entry.path)))
            total += size
    except OSError:
        return 0
    if total <= budget_bytes:
        return 0
    scored.sort(key=lambda t: t[0])  # oldest first
    removed = 0
    for _mtime, size, path in scored:
        if total <= budget_bytes:
            break
        if verbose:
            print(f"  rm {path} (size={size / 1024 / 1024:.0f} MB, over-budget)")
        if _rm(path, dry_run):
            total -= size
            removed += 1
    return removed


def _target_release_has_old_content(target: Path, older_than_days: int = 1) -> bool:
    """Cheap heuristic: does target/release/ hold any file older than N days at depth <=2?"""
    release = target / "release"
    if not release.is_dir():
        return False
    cutoff = time.time() - older_than_days * 86400
    try:
        for entry in os.scandir(release):
            try:
                st = entry.stat(follow_symlinks=False)
            except OSError:
                continue
            if not entry.is_dir(follow_symlinks=False):
                if st.st_mtime < cutoff:
                    return True
                continue
            # Depth 2
            try:
                sub = os.scandir(entry.path)
            except OSError:
                continue
            with sub:
                for child in sub:
                    try:
                        cst = child.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    if cst.st_mtime < cutoff:
                        return True
    except OSError:
        return False
    return False


def clean_cargo_artifacts(
    root: Path, dry_run: bool, verbose: bool
) -> StageResult:
    """Stage D: age-based prune of cargo deps/, build/, .fingerprint/, incremental/."""
    start = time.monotonic()
    target = root / "target"
    if not target.is_dir():
        return StageResult("cargo", 0, time.monotonic() - start, "target/ absent")

    aggressive = _target_release_has_old_content(target, older_than_days=1)
    days = CARGO_AGGRESSIVE_DAYS if aggressive else CARGO_MODERATE_DAYS
    cutoff = time.time() - days * 86400

    removed = 0

    for profile_rel in CARGO_PROFILES:
        profile = root / profile_rel
        if not profile.is_dir():
            continue

        deps = profile / "deps"
        if deps.is_dir():
            try:
                for entry in os.scandir(deps):
                    if not entry.is_file(follow_symlinks=False):
                        continue
                    if not entry.name.endswith(CARGO_DEPS_EXTS):
                        continue
                    try:
                        mtime = entry.stat(follow_symlinks=False).st_mtime
                    except OSError:
                        continue
                    if mtime >= cutoff:
                        continue
                    if verbose:
                        print(f"  rm {entry.path}")
                    if _rm(Path(entry.path), dry_run):
                        removed += 1
            except OSError:
                pass

        for kind in CARGO_KIND_DIRS:
            kind_dir = profile / kind
            if not kind_dir.is_dir():
                continue
            try:
                for entry in os.scandir(kind_dir):
                    if not entry.is_dir(follow_symlinks=False):
                        continue
                    try:
                        mtime = entry.stat(follow_symlinks=False).st_mtime
                    except OSError:
                        continue
                    if mtime >= cutoff:
                        continue
                    if verbose:
                        print(f"  rm {entry.path}")
                    if _rm(Path(entry.path), dry_run):
                        removed += 1
            except OSError:
                pass

    # Aggressive: drop target/doc if nothing recent in it.
    if aggressive:
        doc = target / "doc"
        if doc.is_dir() and _dir_has_no_recent(doc, cutoff):
            if verbose:
                print(f"  rm {doc}")
            if _rm(doc, dry_run):
                removed += 1

    # Size-budget pass: the mtime prune above rarely fires during active
    # dev (every build touches every artifact), so enforce a hard cap per
    # kind directory. Oldest entries get evicted first; newest stay.
    budget_removed = 0
    for profile_rel in CARGO_PROFILES:
        profile = root / profile_rel
        if not profile.is_dir():
            continue

        deps = profile / "deps"
        if deps.is_dir():
            budget_removed += _prune_to_size_budget(
                deps,
                int(CARGO_KIND_BUDGETS_GB["deps"] * 1024 ** 3),
                # Only count/prune the cargo-generated artifact extensions;
                # leave test binaries and other files alone.
                entry_filter=lambda e: (
                    e.is_file(follow_symlinks=False)
                    and e.name.endswith(CARGO_DEPS_EXTS)
                ),
                dry_run=dry_run,
                verbose=verbose,
            )

        for kind in CARGO_KIND_DIRS:
            kind_dir = profile / kind
            if not kind_dir.is_dir():
                continue
            budget_removed += _prune_to_size_budget(
                kind_dir,
                int(CARGO_KIND_BUDGETS_GB[kind] * 1024 ** 3),
                entry_filter=lambda e: e.is_dir(follow_symlinks=False),
                dry_run=dry_run,
                verbose=verbose,
            )

    removed += budget_removed

    detail = f"threshold={days}d {'aggressive' if aggressive else 'moderate'}"
    if budget_removed:
        detail += f", budget={budget_removed}"
    return StageResult("cargo", removed, time.monotonic() - start, detail)


def _dir_has_no_recent(root: Path, cutoff: float) -> bool:
    """True if no file under `root` (depth <= 2) has mtime >= cutoff."""
    try:
        for entry in os.scandir(root):
            try:
                st = entry.stat(follow_symlinks=False)
            except OSError:
                continue
            if st.st_mtime >= cutoff:
                return False
            if entry.is_dir(follow_symlinks=False):
                try:
                    sub = os.scandir(entry.path)
                except OSError:
                    continue
                with sub:
                    for child in sub:
                        try:
                            cst = child.stat(follow_symlinks=False)
                        except OSError:
                            continue
                        if cst.st_mtime >= cutoff:
                            return False
    except OSError:
        return True
    return True


def target_size_gb(root: Path) -> float | None:
    target = root / "target"
    if not target.is_dir():
        return None
    try:
        out = subprocess.run(
            ["du", "-sk", str(target)],
            capture_output=True,
            text=True,
            check=True,
            timeout=60,
        )
    except (subprocess.TimeoutExpired, subprocess.CalledProcessError, FileNotFoundError):
        return None
    try:
        kb = int(out.stdout.split()[0])
    except (ValueError, IndexError):
        return None
    return kb / 1024 / 1024


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=os.getcwd(), help="Project root (default: cwd)")
    parser.add_argument("--tmp-dir", default=os.environ.get("TMPDIR", "/tmp"))
    parser.add_argument("--sockets-dir", default="/tmp/capsem")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--verbose", "-v", action="store_true")
    parser.add_argument("--skip-cargo-prune", action="store_true")
    parser.add_argument("--skip-sockets", action="store_true")
    parser.add_argument("--skip-rootfs", action="store_true")
    parser.add_argument("--skip-tmp", action="store_true")
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    tmp_dir = Path(args.tmp_dir)
    sockets_dir = Path(args.sockets_dir)

    print("=== Pruning stale build artifacts ===")
    total_start = time.monotonic()
    results: list[StageResult] = []

    if not args.skip_rootfs:
        results.append(clean_rootfs_scratch(root, args.dry_run, args.verbose))
    if not args.skip_sockets:
        results.append(clean_orphan_sockets(sockets_dir, args.dry_run, args.verbose))
    if not args.skip_tmp:
        results.append(clean_tmp_fixtures(tmp_dir, args.dry_run, args.verbose))
    if not args.skip_cargo_prune:
        results.append(clean_cargo_artifacts(root, args.dry_run, args.verbose))

    for r in results:
        suffix = f" [{r.detail}]" if r.detail else ""
        print(f"  {r.name:8s} removed={r.removed:<6d} {r.elapsed_s*1000:7.0f} ms{suffix}")

    size_gb = target_size_gb(root)
    if size_gb is not None:
        print(f"  target/ now {size_gb:.1f} GB")

    total = time.monotonic() - total_start
    print(f"=== Done in {total:.1f}s ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
