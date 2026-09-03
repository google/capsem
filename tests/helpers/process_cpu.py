"""High-resolution process CPU accounting for performance gates."""

from __future__ import annotations

import ctypes
import sys
import time
from decimal import Decimal
from typing import Protocol

NANOSECONDS_PER_SECOND = Decimal(1_000_000_000)


class CpuTimes(Protocol):
    @property
    def user(self) -> float: ...

    @property
    def system(self) -> float: ...


class ProcessLike(Protocol):
    @property
    def pid(self) -> int: ...

    def cpu_times(self) -> CpuTimes: ...


def process_cpu_seconds(
    process: ProcessLike,
) -> Decimal:
    """Read monotonic total process CPU with nanosecond precision on Linux.

    Linux's process CPU clock includes every thread, including CPU consumed by
    threads that exit between samples. Summing the live ``schedstat`` files
    does not: the total can move backwards when a Tokio worker exits. Other
    platforms retain psutil's portable user-plus-system counter.
    """
    if sys.platform == "linux":
        runtime_ns = _linux_process_cpu_nanoseconds(process.pid)
        if runtime_ns is not None:
            return Decimal(runtime_ns) / NANOSECONDS_PER_SECOND

    times = process.cpu_times()
    return Decimal(str(times.user)) + Decimal(str(times.system))


def _linux_process_cpu_nanoseconds(pid: int) -> int | None:
    clock_id = ctypes.c_int()
    libc = ctypes.CDLL(None)
    get_clock_id = libc.clock_getcpuclockid
    get_clock_id.argtypes = (ctypes.c_int, ctypes.POINTER(ctypes.c_int))
    get_clock_id.restype = ctypes.c_int
    if get_clock_id(pid, ctypes.byref(clock_id)) != 0:
        return None
    return time.clock_gettime_ns(clock_id.value)
