"""An exclusive gate command refuses a disk its caches can fill.

Every cache honoured its own maximum while the build box's disk filled to 100%
four times in two days (2026-09-27/28), killing gate and release runs with
ENOSPC mid-flight. The budget in `config/cache.toml` is checked before the
machine lock is even waited for, so the refusal costs seconds, not a run.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.cache import budget
from capsem_builder.gate import cachecontrol, preflight
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)
GIB = 1024**3


class ActingRunner(RecordingRunner):
    """A runner that acts, so preflight does what a real run does."""

    observing = False


@pytest.fixture(autouse=True)
def no_machine_lock(monkeypatch) -> None:
    """Never queue behind a real gate: taking the lock is itself the failure."""

    def taken(*_args, **_kwargs):
        raise AssertionError("the machine lock was requested")

    monkeypatch.setattr(preflight.ExclusiveLock, "for_gate", taken)
    monkeypatch.setattr(preflight.snapshot, "digest", lambda *_args: "digest")


def test_an_exclusive_command_refuses_a_disk_its_caches_can_fill(monkeypatch) -> None:
    monkeypatch.delenv("RUNNER_ENVIRONMENT", raising=False)
    monkeypatch.setattr(budget, "filesystem_bytes", lambda _path: 100 * GIB)

    with pytest.raises(GateError, match="do not fit 0.8 of this 100.0 GiB filesystem"):
        with preflight.locked(CONFIG, ActingRunner(ROOT), "test-fast", exclusive=True):
            pytest.fail("the machine lock was taken on a disk the caches can fill")


def test_the_budget_holds_on_a_disk_that_fits(monkeypatch) -> None:
    monkeypatch.delenv("RUNNER_ENVIRONMENT", raising=False)
    monkeypatch.setattr(budget, "filesystem_bytes", lambda _path: 2048 * GIB)

    cachecontrol.verify_disk_budget(CONFIG)


def test_a_hosted_runner_is_exempt(monkeypatch) -> None:
    monkeypatch.setenv("RUNNER_ENVIRONMENT", "github-hosted")
    monkeypatch.setattr(budget, "filesystem_bytes", lambda _path: 14 * GIB)

    cachecontrol.verify_disk_budget(CONFIG)


def test_observation_and_shared_commands_do_not_touch_the_disk(monkeypatch) -> None:
    def refuse(_path: Path) -> int:
        raise AssertionError("inspection read the filesystem")

    monkeypatch.setattr(budget, "filesystem_bytes", refuse)
    with pytest.raises(AssertionError, match="machine lock was requested"):
        with preflight.locked(CONFIG, RecordingRunner(ROOT), "test-fast", exclusive=True):
            pass
    with preflight.locked(CONFIG, ActingRunner(ROOT), "runs", exclusive=False):
        pass
