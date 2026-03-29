"""Shared helpers, constants, and Rich console for all benchmark modules."""

import math
import os

from rich.console import Console

# Rich console writing to stderr (human-readable output)
console = Console(stderr=True)

# Block sizes
BLOCK_1M = 1024 * 1024
BLOCK_4K = 4096

# Disk benchmark defaults
DEFAULT_DIR = "/root"
DEFAULT_SIZE_MB = 256
RAND_IO_SIZE_MB = 64
RAND_IO_COUNT = 10000

# HTTP benchmark defaults
DEFAULT_HTTP_URL = "https://www.google.com/"
DEFAULT_HTTP_N = 50
DEFAULT_HTTP_C = 5


def percentile(sorted_values, pct):
    """Compute the pct-th percentile from a pre-sorted list."""
    if not sorted_values:
        return 0.0
    k = (len(sorted_values) - 1) * pct / 100.0
    f = math.floor(k)
    c = math.ceil(k)
    if f == c:
        return sorted_values[int(k)]
    return sorted_values[f] * (c - k) + sorted_values[c] * (k - f)


def fmt_bytes(n):
    """Format byte count as human-readable string."""
    if n >= 1024 ** 3:
        return f"{n / 1024**3:.1f} GB"
    if n >= 1024 ** 2:
        return f"{n / 1024**2:.1f} MB"
    if n >= 1024:
        return f"{n / 1024:.1f} KB"
    return f"{n} B"


def throughput_mbps(size_bytes, duration_s):
    if duration_s <= 0:
        return 0.0
    return round(size_bytes / (1024 * 1024) / duration_s, 1)


def fdatasync(fd):
    """fdatasync on Linux, fsync fallback on macOS."""
    if hasattr(os, "fdatasync"):
        os.fdatasync(fd)
    else:
        os.fsync(fd)


def drop_caches():
    """Drop page cache so reads hit disk. Requires root."""
    try:
        with open("/proc/sys/vm/drop_caches", "w") as f:
            f.write("3\n")
    except (PermissionError, FileNotFoundError):
        pass
