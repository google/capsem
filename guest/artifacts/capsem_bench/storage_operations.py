"""Existing storage operations; raw observations stay in the same artifact."""
import os
import random
import time

from .helpers import BLOCK_1M, BLOCK_4K, drop_caches, percentile, throughput_mbps
from .io_capture import Operations, raw_summary


def _bench_seq_read_existing(path, size_bytes, drop=False):
    if drop:
        drop_caches()
    observed = Operations((size_bytes + BLOCK_1M - 1) // BLOCK_1M)
    fd = os.open(path, os.O_RDONLY)
    try:
        start = time.monotonic()
        remaining = size_bytes
        while remaining:
            requested = min(BLOCK_1M, remaining)
            op_start = time.monotonic()
            data = os.read(fd, requested)
            observed.record(op_start, requested, len(data))
            remaining -= len(data)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    return {
        "size_bytes": size_bytes,
        "block_size": BLOCK_1M,
        "duration_ms": round(elapsed * 1000, 1),
        "throughput_mbps": throughput_mbps(size_bytes, elapsed),
        **observed.finish(elapsed),
    }


def _bench_rootfs_rand_read(files, count):
    if not files:
        return {"count": 0, "error": "no files found"}
    observed = Operations(count)
    targets = []
    for _ in range(count):
        path, size = random.choice(files)
        max_off = max(size - BLOCK_4K, 0)
        offset = random.randint(0, max_off) & ~(BLOCK_4K - 1)
        targets.append((path, offset, min(BLOCK_4K, size-offset)))
    drop_caches()
    fd_cache = {}
    try:
        start = time.monotonic()
        for path, offset, requested in targets:
            fd = fd_cache.get(path)
            if fd is None:
                fd = os.open(path, os.O_RDONLY)
                fd_cache[path] = fd
            op_start = time.monotonic()
            data = os.pread(fd, requested, offset)
            observed.record(op_start, requested, len(data))
        elapsed = time.monotonic() - start
    finally:
        for fd in fd_cache.values():
            os.close(fd)
    total_bytes = observed.completed_bytes
    return {
        "count": count,
        "files_sampled": len(fd_cache),
        "duration_ms": round(elapsed * 1000, 1),
        "iops": round(count / elapsed, 1) if elapsed > 0 else 0,
        "throughput_mbps": throughput_mbps(total_bytes, elapsed),
        **observed.finish(elapsed),
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
        raw = raw_summary(size_bytes, count, elapsed, latencies)
        if raw is not None:
            summary["raw_io"] = raw
        else:
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
    if size_bytes % block_size:
        raise ValueError('profile bytes must be whole operations')
    observed = Operations(count)
    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        start = time.monotonic()
        for _ in range(count):
            op_start = time.monotonic()
            observed.record(op_start, block_size, os.write(fd, buf))
        os.ftruncate(fd, size_bytes)
        os.fsync(fd)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    observed.finish(elapsed)
    return _io_summary(observed.completed_bytes, block_size, count, elapsed, observed.latencies)


def _bench_seq_read_profile(testfile, size_bytes, block_size, drop=False):
    if drop:
        drop_caches()
    count = (size_bytes + block_size - 1) // block_size
    observed = Operations(count)
    fd = os.open(testfile, os.O_RDONLY)
    try:
        start = time.monotonic()
        remaining = size_bytes
        while remaining:
            requested = min(block_size, remaining)
            op_start = time.monotonic()
            data = os.read(fd, requested)
            observed.record(op_start, requested, len(data))
            remaining -= len(data)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    observed.finish(elapsed)
    return _io_summary(observed.completed_bytes, block_size, count, elapsed, observed.latencies)


def _random_offsets(file_size, op_size, count):
    max_off = max(file_size - op_size, 0)
    return [random.randint(0, max_off) & ~(op_size - 1) for _ in range(count)]


def _bench_random_read_profile(testfile, size_bytes, op_size, count):
    observed = Operations(count, human_latencies=True)
    offsets = _random_offsets(size_bytes, op_size, count)
    drop_caches()
    fd = os.open(testfile, os.O_RDONLY)
    try:
        start = time.monotonic()
        for off in offsets:
            op_start = time.monotonic()
            data = os.pread(fd, op_size, off)
            observed.record(op_start, op_size, len(data))
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    observed.finish(elapsed)
    return _io_summary(observed.completed_bytes, op_size, count, elapsed, observed.latencies)


def _bench_random_write_profile(testfile, size_bytes, op_size, count, sync_each=False):
    observed = Operations(count, human_latencies=True)
    offsets = _random_offsets(size_bytes, op_size, count)
    buf = os.urandom(op_size)
    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT, 0o644)
    try:
        os.ftruncate(fd, size_bytes)
        start = time.monotonic()
        for off in offsets:
            op_start = time.monotonic()
            written = os.pwrite(fd, buf, off)
            if sync_each:
                os.fsync(fd)
            observed.record(op_start, op_size, written)
        if not sync_each:
            os.fsync(fd)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)
    observed.finish(elapsed)
    result = _io_summary(observed.completed_bytes, op_size, count, elapsed, observed.latencies)
    result["sync_each"] = sync_each
    return result


