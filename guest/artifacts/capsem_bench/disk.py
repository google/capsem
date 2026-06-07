"""Scratch disk I/O benchmarks (sequential and random, dd-style)."""

import os
import random
import time

from rich.table import Table
from rich.text import Text

from .helpers import (
    BLOCK_1M, BLOCK_4K, DEFAULT_DIR, DEFAULT_SIZE_MB,
    RAND_IO_COUNT, RAND_IO_SIZE_MB,
    console, fdatasync, drop_caches, throughput_mbps,
)


def bench_seq_write(testfile, size_bytes):
    """Sequential write with 1MB blocks, fdatasync at end."""
    buf = b"\0" * BLOCK_1M
    count = size_bytes // BLOCK_1M

    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        start = time.monotonic()
        for _ in range(count):
            os.write(fd, buf)
        fdatasync(fd)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)

    return {
        "size_bytes": size_bytes,
        "block_size": BLOCK_1M,
        "duration_ms": round(elapsed * 1000, 1),
        "throughput_mbps": throughput_mbps(size_bytes, elapsed),
    }


def bench_seq_read(testfile, size_bytes):
    """Sequential read with 1MB blocks after dropping caches."""
    buf = b"\0" * BLOCK_1M
    count = size_bytes // BLOCK_1M
    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        for _ in range(count):
            os.write(fd, buf)
        fdatasync(fd)
    finally:
        os.close(fd)

    drop_caches()

    fd = os.open(testfile, os.O_RDONLY)
    try:
        start = time.monotonic()
        while True:
            data = os.read(fd, BLOCK_1M)
            if not data:
                break
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)

    return {
        "size_bytes": size_bytes,
        "block_size": BLOCK_1M,
        "duration_ms": round(elapsed * 1000, 1),
        "throughput_mbps": throughput_mbps(size_bytes, elapsed),
    }


def bench_rand_write_4k(testfile):
    """Random 4K writes with fdatasync per write (measures real IOPS)."""
    file_size = RAND_IO_SIZE_MB * 1024 * 1024
    max_offset = file_size - BLOCK_4K
    buf = os.urandom(BLOCK_4K)
    offsets = [random.randint(0, max_offset) & ~(BLOCK_4K - 1)
               for _ in range(RAND_IO_COUNT)]

    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        os.ftruncate(fd, file_size)
        start = time.monotonic()
        for off in offsets:
            os.pwrite(fd, buf, off)
            fdatasync(fd)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)

    total_bytes = RAND_IO_COUNT * BLOCK_4K
    iops = round(RAND_IO_COUNT / elapsed, 1) if elapsed > 0 else 0
    return {
        "count": RAND_IO_COUNT,
        "block_size": BLOCK_4K,
        "duration_ms": round(elapsed * 1000, 1),
        "iops": iops,
        "throughput_mbps": throughput_mbps(total_bytes, elapsed),
    }


def bench_rand_read_4k(testfile):
    """Random 4K reads after dropping caches."""
    file_size = RAND_IO_SIZE_MB * 1024 * 1024
    max_offset = file_size - BLOCK_4K
    offsets = [random.randint(0, max_offset) & ~(BLOCK_4K - 1)
               for _ in range(RAND_IO_COUNT)]

    fd = os.open(testfile, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        os.ftruncate(fd, file_size)
        fdatasync(fd)
    finally:
        os.close(fd)

    drop_caches()

    fd = os.open(testfile, os.O_RDONLY)
    try:
        start = time.monotonic()
        for off in offsets:
            os.pread(fd, BLOCK_4K, off)
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)

    total_bytes = RAND_IO_COUNT * BLOCK_4K
    iops = round(RAND_IO_COUNT / elapsed, 1) if elapsed > 0 else 0
    return {
        "count": RAND_IO_COUNT,
        "block_size": BLOCK_4K,
        "duration_ms": round(elapsed * 1000, 1),
        "iops": iops,
        "throughput_mbps": throughput_mbps(total_bytes, elapsed),
    }


def disk_bench(directory=None, size_mb=None):
    """Run scratch disk I/O benchmarks."""
    directory = directory or os.environ.get("CAPSEM_BENCH_DIR", DEFAULT_DIR)
    size_mb = size_mb or int(os.environ.get("CAPSEM_BENCH_SIZE_MB", DEFAULT_SIZE_MB))
    size_bytes = size_mb * 1024 * 1024
    testfile = os.path.join(directory, ".capsem-bench-test")

    table = Table(title=Text(f"Scratch Disk I/O  [{directory}, {size_mb} MB]"))
    table.add_column("Test", style="bold")
    table.add_column("Throughput", justify="right")
    table.add_column("IOPS", justify="right")
    table.add_column("Duration", justify="right")

    results = {"directory": directory, "size_mb": size_mb}

    try:
        stats = bench_seq_write(testfile, size_bytes)
        results["seq_write"] = stats
        table.add_row("Seq write (1MB)", f"{stats['throughput_mbps']} MB/s",
                       "-", f"{stats['duration_ms']} ms")

        stats = bench_seq_read(testfile, size_bytes)
        results["seq_read"] = stats
        table.add_row("Seq read (1MB)", f"{stats['throughput_mbps']} MB/s",
                       "-", f"{stats['duration_ms']} ms")

        stats = bench_rand_write_4k(testfile)
        results["rand_write_4k"] = stats
        table.add_row("Rand write (4K)", f"{stats['throughput_mbps']} MB/s",
                       f"{stats['iops']:.0f}", f"{stats['duration_ms']} ms")

        stats = bench_rand_read_4k(testfile)
        results["rand_read_4k"] = stats
        table.add_row("Rand read (4K)", f"{stats['throughput_mbps']} MB/s",
                       f"{stats['iops']:.0f}", f"{stats['duration_ms']} ms")
    finally:
        try:
            os.unlink(testfile)
        except OSError:
            pass

    console.print(table)
    return results
