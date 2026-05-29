"""Storage-path diagnostics for rootfs, workspace, overlay, and tmpfs."""

import os
import random
import stat
import time

from rich.table import Table
from rich.text import Text

from .disk import (
    bench_rand_read_4k,
    bench_rand_write_4k,
    bench_seq_read,
    bench_seq_write,
)
from .helpers import (
    BLOCK_1M,
    BLOCK_4K,
    console,
    drop_caches,
    fmt_bytes,
    throughput_mbps,
)
from .rootfs import ROOTFS_SCAN_DIRS, collect_rootfs_files, find_largest_file

DEFAULT_STORAGE_PATHS = ["/root", "/tmp", "/var/tmp", "/var/log", "/run"]
DEFAULT_STORAGE_SIZE_MB = 64
ROOTFS_READ_FILES = ["/bin/bash", "/usr/bin/python3", "/usr/bin/node"]
ROOTFS_RAND_COUNT = 2000


def parse_mountinfo(text):
    """Parse Linux /proc/self/mountinfo into a compact dict list."""
    mounts = []
    for line in text.splitlines():
        if " - " not in line:
            continue
        left, right = line.split(" - ", 1)
        left_parts = left.split()
        right_parts = right.split()
        if len(left_parts) < 5 or len(right_parts) < 3:
            continue
        mounts.append({
            "mount_point": left_parts[4],
            "root": left_parts[3],
            "fs_type": right_parts[0],
            "source": right_parts[1],
            "options": right_parts[2],
        })
    return mounts


def read_mountinfo():
    try:
        with open("/proc/self/mountinfo") as f:
            return parse_mountinfo(f.read())
    except OSError:
        return []


def find_mount_for_path(path, mounts):
    """Return the most specific mount containing path."""
    real = os.path.realpath(path)
    best = None
    best_len = -1
    for mount in mounts:
        mount_point = mount.get("mount_point", "")
        if real == mount_point or real.startswith(mount_point.rstrip("/") + "/"):
            if len(mount_point) > best_len:
                best = mount
                best_len = len(mount_point)
    return best or {}


def path_stat(path, mounts):
    info = {
        "path": path,
        "exists": os.path.exists(path),
        "writable": os.access(path, os.W_OK),
        "mount": find_mount_for_path(path, mounts),
    }
    if not info["exists"]:
        return info
    st = os.stat(path)
    vfs = os.statvfs(path)
    info["mode"] = stat.filemode(st.st_mode)
    info["statvfs"] = {
        "block_size": vfs.f_bsize,
        "fragment_size": vfs.f_frsize,
        "blocks": vfs.f_blocks,
        "blocks_free": vfs.f_bfree,
        "blocks_available": vfs.f_bavail,
        "files": vfs.f_files,
        "files_free": vfs.f_ffree,
    }
    return info


def storage_paths():
    raw = os.environ.get("CAPSEM_STORAGE_BENCH_PATHS")
    paths = raw.split(":") if raw else DEFAULT_STORAGE_PATHS
    seen = set()
    deduped = []
    for path in paths:
        path = path.strip()
        if path and path not in seen:
            seen.add(path)
            deduped.append(path)
    return deduped


def writable_path_bench(path, size_mb=None):
    size_mb = size_mb or int(
        os.environ.get("CAPSEM_STORAGE_BENCH_SIZE_MB", DEFAULT_STORAGE_SIZE_MB)
    )
    size_bytes = size_mb * 1024 * 1024
    testfile = os.path.join(path, ".capsem-storage-bench")
    result = {"path": path, "size_mb": size_mb}
    try:
        result["seq_write"] = bench_seq_write(testfile, size_bytes)
        result["seq_read_cold"] = bench_seq_read(testfile, size_bytes)
        result["seq_read_warm"] = _bench_seq_read_existing(testfile, size_bytes)
        result["rand_write_4k"] = bench_rand_write_4k(testfile)
        result["rand_read_4k"] = bench_rand_read_4k(testfile)
    except OSError as exc:
        result["error"] = str(exc)
    finally:
        try:
            os.unlink(testfile)
        except OSError:
            pass
    return result


def rootfs_storage_bench():
    mounts = read_mountinfo()
    largest_path, largest_size = find_largest_file(ROOTFS_SCAN_DIRS)
    files = collect_rootfs_files(ROOTFS_SCAN_DIRS)
    result = {
        "scan_dirs": ROOTFS_SCAN_DIRS,
        "files_found": len(files),
        "largest_file": largest_path,
        "largest_file_size": largest_size,
    }
    candidates = []
    if largest_path:
        candidates.append((largest_path, largest_size, "largest"))
    for path in ROOTFS_READ_FILES:
        if os.path.exists(path):
            candidates.append((path, os.path.getsize(path), os.path.basename(path)))

    seq = []
    for path, size, label in candidates:
        cold = _bench_seq_read_existing(path, size, drop=True)
        warm = _bench_seq_read_existing(path, size, drop=False)
        seq.append({
            "label": label,
            "path": path,
            "size_bytes": size,
            "mount": find_mount_for_path(path, mounts),
            "cold": cold,
            "warm": warm,
        })
    result["seq_reads"] = seq
    result["rand_read_4k"] = _bench_rootfs_rand_read(files, ROOTFS_RAND_COUNT)
    return result


def _bench_seq_read_existing(path, size_bytes, drop=False):
    if drop:
        drop_caches()
    fd = os.open(path, os.O_RDONLY)
    try:
        start = time.monotonic()
        while os.read(fd, BLOCK_1M):
            pass
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    return {
        "size_bytes": size_bytes,
        "block_size": BLOCK_1M,
        "duration_ms": round(elapsed * 1000, 1),
        "throughput_mbps": throughput_mbps(size_bytes, elapsed),
    }


def _bench_rootfs_rand_read(files, count):
    if not files:
        return {"count": 0, "error": "no files found"}
    targets = []
    for _ in range(count):
        path, size = random.choice(files)
        max_off = max(size - BLOCK_4K, 0)
        offset = random.randint(0, max_off) & ~(BLOCK_4K - 1)
        targets.append((path, offset))
    drop_caches()
    fd_cache = {}
    try:
        start = time.monotonic()
        for path, offset in targets:
            fd = fd_cache.get(path)
            if fd is None:
                fd = os.open(path, os.O_RDONLY)
                fd_cache[path] = fd
            os.pread(fd, BLOCK_4K, offset)
        elapsed = time.monotonic() - start
    finally:
        for fd in fd_cache.values():
            os.close(fd)
    total_bytes = count * BLOCK_4K
    return {
        "count": count,
        "files_sampled": len(fd_cache),
        "duration_ms": round(elapsed * 1000, 1),
        "iops": round(count / elapsed, 1) if elapsed > 0 else 0,
        "throughput_mbps": throughput_mbps(total_bytes, elapsed),
    }


def storage_bench():
    """Run storage diagnostics across rootfs and writable guest paths."""
    mounts = read_mountinfo()
    paths = storage_paths()
    results = {
        "mounts": mounts,
        "paths": {
            path: path_stat(path, mounts) for path in ["/", *paths, *ROOTFS_SCAN_DIRS]
        },
        "rootfs": rootfs_storage_bench(),
        "writable": {},
    }

    for path in paths:
        if os.path.isdir(path) and os.access(path, os.W_OK):
            results["writable"][path] = writable_path_bench(path)
        else:
            results["writable"][path] = {
                "path": path,
                "skipped": "not writable directory",
            }

    _print_storage_summary(results)
    return results


def _print_storage_summary(results):
    table = Table(title=Text("Storage Path Diagnostics"))
    table.add_column("Path", style="bold")
    table.add_column("FS")
    table.add_column("Write", justify="right")
    table.add_column("Cold Read", justify="right")
    table.add_column("Warm Read", justify="right")
    table.add_column("Rand Read", justify="right")

    for path, stats in results["writable"].items():
        fs_type = results["paths"].get(path, {}).get("mount", {}).get("fs_type", "?")
        if "error" in stats or "skipped" in stats:
            table.add_row(
                path,
                fs_type,
                stats.get("error") or stats.get("skipped"),
                "-",
                "-",
                "-",
            )
            continue
        table.add_row(
            path,
            fs_type,
            f"{stats['seq_write']['throughput_mbps']} MB/s",
            f"{stats['seq_read_cold']['throughput_mbps']} MB/s",
            f"{stats['seq_read_warm']['throughput_mbps']} MB/s",
            f"{stats['rand_read_4k']['iops']:.0f} IOPS",
        )

    for item in results["rootfs"]["seq_reads"]:
        fs_type = item.get("mount", {}).get("fs_type", "?")
        label = f"rootfs:{item['label']} ({fmt_bytes(item['size_bytes'])})"
        table.add_row(
            label,
            fs_type,
            "-",
            f"{item['cold']['throughput_mbps']} MB/s",
            f"{item['warm']['throughput_mbps']} MB/s",
            "-",
        )

    console.print(table)
