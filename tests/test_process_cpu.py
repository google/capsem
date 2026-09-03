from __future__ import annotations

from pathlib import Path
from typing import NamedTuple

from helpers.process_cpu import process_cpu_seconds


class FakeCpuTimes(NamedTuple):
    user: float
    system: float


class FakeProcess:
    pid = 42

    def cpu_times(self) -> FakeCpuTimes:
        return FakeCpuTimes(user=1.25, system=0.5)


def test_process_cpu_sums_every_linux_thread(tmp_path: Path) -> None:
    task_root = tmp_path / "42" / "task"
    for task_id, runtime_ns in (("42", 125_000_000), ("43", 250_000_000)):
        task_dir = task_root / task_id
        task_dir.mkdir(parents=True)
        (task_dir / "schedstat").write_text(
            f"{runtime_ns} 99 3\n",
            encoding="utf-8",
        )

    assert process_cpu_seconds(FakeProcess(), proc_root=tmp_path) == 0.375


def test_process_cpu_uses_portable_counter_without_procfs(tmp_path: Path) -> None:
    assert process_cpu_seconds(FakeProcess(), proc_root=tmp_path) == 1.75


def test_process_cpu_rejects_malformed_scheduler_accounting(tmp_path: Path) -> None:
    task_dir = tmp_path / "42" / "task" / "42"
    task_dir.mkdir(parents=True)
    (task_dir / "schedstat").write_text("\n", encoding="utf-8")

    try:
        process_cpu_seconds(FakeProcess(), proc_root=tmp_path)
    except ValueError as error:
        assert "empty scheduler accounting file" in str(error)
        return

    raise AssertionError("malformed scheduler accounting was accepted")
