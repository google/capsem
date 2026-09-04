from __future__ import annotations

import os
import threading
import time
from typing import NamedTuple
from unittest.mock import patch

from helpers.process_cpu import process_cpu_seconds


class FakeCpuTimes(NamedTuple):
    user: float
    system: float


class FakeProcess:
    pid = 42

    def cpu_times(self) -> FakeCpuTimes:
        return FakeCpuTimes(user=1.25, system=0.5)


class CurrentProcess:
    pid = os.getpid()

    def cpu_times(self) -> FakeCpuTimes:
        return FakeCpuTimes(user=time.process_time(), system=0.0)


def test_process_cpu_retains_cpu_from_exited_threads() -> None:
    before = process_cpu_seconds(CurrentProcess())

    def consume_thread_cpu() -> None:
        started = time.thread_time()
        while time.thread_time() - started < 0.02:
            pass

    worker = threading.Thread(target=consume_thread_cpu)
    worker.start()
    worker.join()

    assert process_cpu_seconds(CurrentProcess()) - before >= 0.015


def test_process_cpu_uses_portable_counter_when_process_clock_is_unavailable() -> None:
    with patch("helpers.process_cpu._linux_process_cpu_nanoseconds", return_value=None):
        assert process_cpu_seconds(FakeProcess()) == 1.75
