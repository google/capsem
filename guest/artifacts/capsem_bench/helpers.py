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

# Local/external-network benchmark selection.
LOCAL_MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"
ALLOW_PUBLIC_NETWORK_ENV = "CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK"
PUBLIC_HTTP_URL = "https://www.google.com/"

# HTTP benchmark defaults. The external URL is only used when
# CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK=1; default release gates should use the
# deterministic local lab or skip cleanly.
DEFAULT_HTTP_URL = None
DEFAULT_HTTP_N = 50
DEFAULT_HTTP_C = 5


def local_mock_server_url(path):
    base_url = os.environ.get(LOCAL_MOCK_SERVER_ENV)
    if not base_url:
        return None
    return f"{base_url.rstrip('/')}/{path.lstrip('/')}"


def public_network_allowed():
    return os.environ.get(ALLOW_PUBLIC_NETWORK_ENV) == "1"


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
