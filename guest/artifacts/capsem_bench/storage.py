"""Storage-path diagnostics for rootfs, workspace, overlay, and tmpfs."""

import os
import random
import stat
import struct
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
    percentile,
    throughput_mbps,
)
from .rootfs import ROOTFS_SCAN_DIRS, collect_rootfs_files, find_largest_file

DEFAULT_STORAGE_PATHS = ["/root", "/tmp", "/var/tmp", "/var/log", "/run"]
DEFAULT_STORAGE_SIZE_MB = 64
DEFAULT_IO_PROFILE_SIZE_MB = 64
DEFAULT_IO_PROFILE_RANDOM_OPS = 2000
IO_PROFILE_BLOCK_SIZES = (BLOCK_4K, 64 * 1024, BLOCK_1M)
ROOTFS_READ_FILES = ["/bin/bash", "/usr/bin/python3", "/usr/bin/node"]
ROOTFS_RAND_COUNT = 2000
SQUASHFS_MAGIC = 0x73717368
SQUASHFS_COMPRESSIONS = {
    1: "gzip",
    2: "lzma",
    3: "lzo",
    4: "xz",
    5: "lz4",
    6: "zstd",
}


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


def parse_mount_options(options):
    parsed = {}
    for option in options.split(","):
        key, sep, value = option.partition("=")
        parsed[key] = value if sep else True
    return parsed


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
        result["io_profile"] = io_profile_bench(path)
    except OSError as exc:
        result["error"] = str(exc)
    finally:
        try:
            os.unlink(testfile)
        except OSError:
            pass
    return result


def io_profile_bench(
    path,
    *,
    size_mb=None,
    seq_block_sizes=IO_PROFILE_BLOCK_SIZES,
    rand_op_count=None,
):
    size_mb = size_mb or int(
        os.environ.get("CAPSEM_STORAGE_IO_PROFILE_SIZE_MB", DEFAULT_IO_PROFILE_SIZE_MB)
    )
    rand_op_count = rand_op_count or int(
        os.environ.get("CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS", DEFAULT_IO_PROFILE_RANDOM_OPS)
    )
    size_bytes = size_mb * 1024 * 1024
    testfile = os.path.join(path, ".capsem-storage-io-profile")
    result = {
        "path": path,
        "size_mb": size_mb,
        "random_ops": rand_op_count,
        "sequential": {},
        "random": {},
    }

    try:
        for block_size in seq_block_sizes:
            key = _block_key(block_size)
            result["sequential"][key] = {
                "write": _bench_seq_write_profile(testfile, size_bytes, block_size),
                "read_cold": _bench_seq_read_profile(
                    testfile, size_bytes, block_size, drop=True
                ),
                "read_warm": _bench_seq_read_profile(
                    testfile, size_bytes, block_size, drop=False
                ),
            }

        result["random"]["read_4k"] = _bench_random_read_profile(
            testfile, size_bytes, BLOCK_4K, rand_op_count
        )
        result["random"]["write_4k_sync"] = _bench_random_write_profile(
            testfile, size_bytes, BLOCK_4K, rand_op_count, sync_each=True
        )
    finally:
        try:
            os.unlink(testfile)
        except OSError:
            pass

    return result


def parse_squashfs_superblock(data, device="/dev/vda"):
    if len(data) < 32:
        return {"device": device, "error": "short squashfs superblock"}

    (
        magic,
        inodes,
        mkfs_time,
        block_size,
        fragments,
        compression_id,
        block_log,
        flags,
        no_ids,
        major,
        minor,
    ) = struct.unpack_from("<IIIIIHHHHHH", data, 0)

    if magic != SQUASHFS_MAGIC:
        return {
            "device": device,
            "magic": f"0x{magic:08x}",
            "error": "not squashfs",
        }

    return {
        "device": device,
        "magic": f"0x{magic:08x}",
        "version": f"{major}.{minor}",
        "compression_id": compression_id,
        "compression": SQUASHFS_COMPRESSIONS.get(
            compression_id, f"unknown:{compression_id}"
        ),
        "block_size_bytes": block_size,
        "block_size": fmt_bytes(block_size),
        "block_log": block_log,
        "flags": flags,
        "inodes": inodes,
        "fragments": fragments,
        "mkfs_time": mkfs_time,
        "id_count": no_ids,
    }


def read_squashfs_superblock(device="/dev/vda"):
    try:
        with open(device, "rb") as f:
            info = parse_squashfs_superblock(f.read(96), device=device)
    except OSError as exc:
        info = {"device": device, "error": str(exc)}

    sys_name = os.path.basename(device)
    read_ahead = f"/sys/block/{sys_name}/queue/read_ahead_kb"
    try:
        with open(read_ahead) as f:
            info["read_ahead_kb"] = int(f.read().strip())
    except (OSError, ValueError):
        pass
    return info


def _read_text(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def _read_int(path):
    value = _read_text(path)
    if value is None:
        return None
    try:
        return int(value)
    except ValueError:
        return value


def rootfs_backing_metadata(mounts):
    root_mount = find_mount_for_path("/", mounts)
    root_options = parse_mount_options(root_mount.get("options", ""))
    squashfs_mounts = [
        mount for mount in mounts if mount.get("fs_type") == "squashfs"
    ]
    return {
        "root_mount": root_mount,
        "overlay_lowerdir": root_options.get("lowerdir"),
        "overlay_upperdir": root_options.get("upperdir"),
        "overlay_workdir": root_options.get("workdir"),
        "squashfs_mounts": squashfs_mounts,
        "squashfs_superblock": read_squashfs_superblock("/dev/vda"),
    }


def read_kernel_cmdline(path="/proc/cmdline"):
    text = _read_text(path) or ""
    return {
        "raw": text,
        "args": text.split(),
    }


def read_block_queues(sys_block="/sys/block"):
    queues = {}
    try:
        devices = sorted(os.listdir(sys_block))
    except OSError:
        return queues

    fields = (
        "scheduler",
        "read_ahead_kb",
        "nr_requests",
        "rotational",
        "logical_block_size",
        "physical_block_size",
        "max_sectors_kb",
        "nomerges",
        "rq_affinity",
        "io_poll",
    )
    for device in devices:
        if not device.startswith("vd"):
            continue
        queue_dir = os.path.join(sys_block, device, "queue")
        info = {}
        for field in fields:
            value = _read_int(os.path.join(queue_dir, field))
            if value is not None:
                info[field] = value
        if "scheduler" in info:
            selected = _selected_scheduler(str(info["scheduler"]))
            if selected:
                info["selected_scheduler"] = selected
        queues[device] = info
    return queues


def _selected_scheduler(value):
    for part in value.split():
        if part.startswith("[") and part.endswith("]"):
            return part[1:-1]
    return None


def read_fuse_connections(sys_fuse="/sys/fs/fuse/connections"):
    connections = {}
    try:
        conn_ids = sorted(os.listdir(sys_fuse), key=lambda item: int(item))
    except (OSError, ValueError):
        return connections

    for conn_id in conn_ids:
        conn_dir = os.path.join(sys_fuse, conn_id)
        info = {}
        for field in ("max_background", "congestion_threshold", "waiting"):
            value = _read_int(os.path.join(conn_dir, field))
            if value is not None:
                info[field] = value
        connections[conn_id] = info
    return connections


def kernel_storage_context():
    return {
        "cmdline": read_kernel_cmdline(),
        "block_queues": read_block_queues(),
        "fuse_connections": read_fuse_connections(),
        "known_host_queue_sizes": {
            "kvm_virtio_blk": 256,
            "kvm_virtio_fs": [256, 256],
        },
    }


def rootfs_storage_bench():
    mounts = read_mountinfo()
    largest_path, largest_size = find_largest_file(ROOTFS_SCAN_DIRS)
    files = collect_rootfs_files(ROOTFS_SCAN_DIRS)
    result = {
        "scan_dirs": ROOTFS_SCAN_DIRS,
        "files_found": len(files),
        "largest_file": largest_path,
        "largest_file_size": largest_size,
        "backing": rootfs_backing_metadata(mounts),
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


def _block_key(size):
    if size == BLOCK_4K:
        return "4k"
    if size == 64 * 1024:
        return "64k"
    if size == BLOCK_1M:
        return "1m"
    return str(size)


def _io_summary(size_bytes, block_size, count, elapsed, latencies=None):
    summary = {
        "size_bytes": size_bytes,
        "block_size": block_size,
        "count": count,
        "duration_ms": round(elapsed * 1000, 1),
        "iops": round(count / elapsed, 1) if elapsed > 0 else 0,
        "throughput_mbps": throughput_mbps(size_bytes, elapsed),
        "avg_latency_ms": round((elapsed * 1000) / count, 3) if count else 0,
    }
    if latencies:
        ordered = sorted(latencies)
        summary["latency_ms"] = {
            "p50": round(percentile(ordered, 50), 3),
            "p95": round(percentile(ordered, 95), 3),
            "p99": round(percentile(ordered, 99), 3),
            "max": round(ordered[-1], 3),
        }
    return summary


def _bench_seq_write_profile(testfile, size_bytes, block_size):
    buf = b"\0" * block_size
    count = size_bytes // block_size
    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        start = time.monotonic()
        for _ in range(count):
            os.write(fd, buf)
        os.ftruncate(fd, size_bytes)
        os.fsync(fd)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    return _io_summary(size_bytes, block_size, count, elapsed)


def _bench_seq_read_profile(testfile, size_bytes, block_size, drop=False):
    if drop:
        drop_caches()
    count = 0
    fd = os.open(testfile, os.O_RDONLY)
    try:
        start = time.monotonic()
        while os.read(fd, block_size):
            count += 1
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    return _io_summary(size_bytes, block_size, count, elapsed)


def _random_offsets(file_size, op_size, count):
    max_off = max(file_size - op_size, 0)
    return [random.randint(0, max_off) & ~(op_size - 1) for _ in range(count)]


def _bench_random_read_profile(testfile, size_bytes, op_size, count):
    offsets = _random_offsets(size_bytes, op_size, count)
    drop_caches()
    latencies = []
    fd = os.open(testfile, os.O_RDONLY)
    try:
        start = time.monotonic()
        for off in offsets:
            op_start = time.monotonic()
            os.pread(fd, op_size, off)
            latencies.append((time.monotonic() - op_start) * 1000)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    return _io_summary(count * op_size, op_size, count, elapsed, latencies)


def _bench_random_write_profile(testfile, size_bytes, op_size, count, sync_each=False):
    offsets = _random_offsets(size_bytes, op_size, count)
    buf = os.urandom(op_size)
    latencies = []
    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT, 0o644)
    try:
        os.ftruncate(fd, size_bytes)
        start = time.monotonic()
        for off in offsets:
            op_start = time.monotonic()
            os.pwrite(fd, buf, off)
            if sync_each:
                os.fsync(fd)
            latencies.append((time.monotonic() - op_start) * 1000)
        if not sync_each:
            os.fsync(fd)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    result = _io_summary(count * op_size, op_size, count, elapsed, latencies)
    result["sync_each"] = sync_each
    return result


def storage_bench():
    """Run storage diagnostics across rootfs and writable guest paths."""
    mounts = read_mountinfo()
    paths = storage_paths()
    results = {
        "kernel": kernel_storage_context(),
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
    table.add_column("Rand Write", justify="right")

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
            f"{stats['rand_write_4k']['iops']:.0f} IOPS",
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
            "-",
        )

    console.print(table)

    profile_table = Table(title=Text("Storage I/O Profile"))
    profile_table.add_column("Path", style="bold")
    profile_table.add_column("Workload")
    profile_table.add_column("Block")
    profile_table.add_column("IOPS", justify="right")
    profile_table.add_column("Throughput", justify="right")
    profile_table.add_column("Avg Lat", justify="right")
    profile_table.add_column("P95 Lat", justify="right")

    for path, stats in results["writable"].items():
        profile = stats.get("io_profile")
        if not profile:
            continue
        for block, seq in profile["sequential"].items():
            for workload in ("write", "read_cold", "read_warm"):
                item = seq[workload]
                profile_table.add_row(
                    path,
                    f"seq_{workload}",
                    block,
                    f"{item['iops']:.0f}",
                    f"{item['throughput_mbps']} MB/s",
                    f"{item['avg_latency_ms']} ms",
                    "-",
                )
        for workload, item in profile["random"].items():
            lat = item.get("latency_ms", {})
            profile_table.add_row(
                path,
                workload,
                _block_key(item["block_size"]),
                f"{item['iops']:.0f}",
                f"{item['throughput_mbps']} MB/s",
                f"{item['avg_latency_ms']} ms",
                f"{lat.get('p95', 0)} ms",
            )

    console.print(profile_table)
