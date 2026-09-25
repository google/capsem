"""A direct `cargo` shares the machine with the gate, so it takes the gate's lock.

The machine lock coordinated `capsem-gate` processes and nothing else. The
mandated wrapper for direct commands routed every worktree's `cargo` into the
one shared target directory and onto every core, arbitrated by nobody: two
sessions deadlocked on an incremental session lock at 0% CPU, a compile
starved another session's VM boot deadlines, and agents invented a
"starting VMs" / "VMs done" protocol over chat to cope. One session sat paused
for ninety minutes of a working day on that etiquette.

Every test here drives the real launcher in a subprocess against a fake
`cargo`, with `HOME` moved so the user-scoped lock resolves under `tmp_path`.
"""

from __future__ import annotations

import contextlib
import json
import os
import stat
import subprocess
import sys
import textwrap
import time
from collections.abc import Iterator
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[3]
BOUNDED = ROOT / "build_system/scripts/ci/run-bounded-command.py"
CONFIG = gate_config.load(ROOT)
LOCK = CONFIG.locks.gate
LEASE = CONFIG.locks.bounded
HOLDER = "capsem-gate focus-test"

HOLD_IT = """
    import fcntl, os, sys, time
    fd = os.open(sys.argv[1], os.O_RDWR | os.O_CREAT, 0o644)
    fcntl.flock(fd, fcntl.LOCK_EX)
    print("held", flush=True)
    time.sleep(float(sys.argv[2]))
"""

FAKE_CARGO = """\
#!/bin/sh
echo "cargo ran: $*"
if [ -n "$FAKE_CARGO_RECORD" ]; then cat "$FAKE_CARGO_RECORD"; fi
if [ -n "$FAKE_CARGO_ENV" ]; then env; fi
"""


def _home_path(home: Path, configured: str) -> Path:
    assert configured.startswith("~/"), "the machine lock must stay user-scoped"
    return home / configured[2:]


def _environment(tmp_path: Path, **extra: str) -> dict[str, str]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    cargo = bin_dir / "cargo"
    cargo.write_text(FAKE_CARGO, encoding="utf-8")
    cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
    inherited = {name: value for name, value in os.environ.items() if name != LOCK.run_marker}
    return {
        **inherited,
        "HOME": str(tmp_path),
        "CAPSEM_CACHE_AUTHORITY": str(tmp_path),
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        **extra,
    }


@contextlib.contextmanager
def _bounded(tmp_path: Path, *command: str, **extra: str) -> Iterator[subprocess.Popen[str]]:
    """The real launcher around `command`; killed and drained on the way out."""
    with subprocess.Popen(
        [sys.executable, str(BOUNDED), "--timeout-seconds", "30", "--", *command],
        env=_environment(tmp_path, **extra),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    ) as process:
        try:
            yield process
        finally:
            process.kill()


@contextlib.contextmanager
def _gate_holds_the_machine(tmp_path: Path) -> Iterator[None]:
    """A separate process that has the lock, and says who it is, until exit."""
    lockfile = _home_path(tmp_path, LOCK.path)
    lockfile.parent.mkdir(parents=True, exist_ok=True)
    with subprocess.Popen(
        [sys.executable, "-c", textwrap.dedent(HOLD_IT), str(lockfile), "60"],
        stdout=subprocess.PIPE,
        text=True,
    ) as holder:
        try:
            assert holder.stdout is not None
            assert holder.stdout.readline().strip() == "held"
            _home_path(tmp_path, LOCK.holder_record).write_text(
                json.dumps(
                    {"pid": holder.pid, "purpose": HOLDER, "started": time.time(), "host": "here"}
                ),
                encoding="utf-8",
            )
            yield
        finally:
            holder.kill()


def _finished_within(process: subprocess.Popen[str], seconds: float) -> bool:
    try:
        process.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        return False
    return True


def test_cargo_waits_for_a_running_gate_and_says_who_it_waits_for(tmp_path: Path) -> None:
    with _bounded(tmp_path, "cargo", "test", "-p", "capsem-core") as waiting:
        with _gate_holds_the_machine(tmp_path):
            assert not _finished_within(waiting, LOCK.report_after_seconds + 3), (
                "cargo ran while a gate held the machine"
            )
        out, err = waiting.communicate(timeout=20)
    assert waiting.returncode == 0, err
    assert "cargo ran: test -p capsem-core" in out
    assert HOLDER in err, "contention must name the holder, on stderr"
    assert "waiting" not in out, "a queue notice must never enter the command's stdout"


def test_a_running_cargo_is_named_to_whoever_arrives_next(tmp_path: Path) -> None:
    record = _home_path(tmp_path, LOCK.holder_record)
    with _bounded(tmp_path, "cargo", "check", FAKE_CARGO_RECORD=str(record)) as done:
        out, err = done.communicate(timeout=30)
    assert done.returncode == 0, err
    seen = json.loads(out.split("\n", 1)[1])
    assert "cargo check" in seen["purpose"]
    assert not record.exists(), "the holder record must not outlive the command"


def test_a_command_that_is_not_machine_work_never_queues(tmp_path: Path) -> None:
    with (
        _gate_holds_the_machine(tmp_path),
        _bounded(tmp_path, sys.executable, "-c", "print('ran')") as free,
    ):
        assert _finished_within(free, 20)
        assert free.returncode == 0


def test_cargo_subcommands_that_neither_compile_nor_write_never_queue(tmp_path: Path) -> None:
    assert LEASE.exempt_subcommands, "the exemption list is empty, so `cargo fmt` queues"
    with _gate_holds_the_machine(tmp_path):
        for subcommand in LEASE.exempt_subcommands:
            with _bounded(tmp_path, "cargo", subcommand) as free:
                assert _finished_within(free, 20), f"`cargo {subcommand}` queued behind a gate"
                assert free.returncode == 0


def test_a_wrapper_or_assignment_does_not_hide_cargo(tmp_path: Path) -> None:
    with (
        _gate_holds_the_machine(tmp_path),
        _bounded(tmp_path, "env", "RUST_LOG=debug", "cargo", "+nightly", "build") as hidden,
    ):
        assert not _finished_within(hidden, LOCK.report_after_seconds + 3)


def test_direct_cargo_enforces_its_cache_contract_inside_the_machine_lease(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import boundedlease

    enforced: list[tuple[Path, tuple[str, ...]]] = []
    monkeypatch.setattr(
        boundedlease,
        "_enforce_cargo_cache",
        lambda root, command: enforced.append((root, tuple(command))),
    )
    monkeypatch.setenv("HOME", str(tmp_path))

    with boundedlease.leased(("cargo", "test", "-p", "capsem-core"), ROOT, {}):
        assert enforced == [(ROOT, ("cargo", "test", "-p", "capsem-core"))]


def test_zero_byte_expiry_does_not_claim_cargo_exceeded_its_limit() -> None:
    from capsem_builder.cache.enforcement import EnforcementResult
    from capsem_builder.gate.boundedlease import _maintenance_notice

    result = EnforcementResult(
        cache_id="cargo",
        before_size_bytes=100,
        after_size_bytes=100,
        pruned=True,
        reclaim_bytes=0,
        action_count=17,
        violations=(),
    )
    assert _maintenance_notice(result) == "Cargo cache maintenance applied 17 prune actions; owned usage 100 -> 100 bytes"


def test_cargo_in_an_argument_is_not_cargo_in_command_position(tmp_path: Path) -> None:
    with (
        _gate_holds_the_machine(tmp_path),
        _bounded(tmp_path, sys.executable, "-c", "print('ran')", "cargo", "build") as free,
    ):
        assert _finished_within(free, 20)


def test_cargo_inside_a_gate_run_does_not_wait_for_its_own_parent(tmp_path: Path) -> None:
    inside_a_run = {LOCK.run_marker: "capsem-gate candidate"}
    with (
        _gate_holds_the_machine(tmp_path),
        _bounded(tmp_path, "cargo", "build", **inside_a_run) as inside,
    ):
        assert _finished_within(inside, 20), "a gate's own cargo deadlocked on its parent's lock"
        assert inside.returncode == 0


def test_the_child_is_told_it_is_inside_a_run(tmp_path: Path) -> None:
    """Or a `capsem-gate` the child starts waits two hours for its own parent."""
    with _bounded(tmp_path, "cargo", "run", FAKE_CARGO_ENV="1") as told:
        out, err = told.communicate(timeout=30)
    assert told.returncode == 0, err
    assert f"{LOCK.run_marker}=" in out


def test_a_wait_that_runs_out_is_not_mistaken_for_the_commands_own_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import boundedlease
    from capsem_builder.gate.tools.ci import run_bounded_command

    impatient = CONFIG.model_copy(
        update={
            "locks": CONFIG.locks.model_copy(
                update={
                    "gate": LOCK.model_copy(
                        update={"wait_timeout_seconds": 0.2, "poll_interval_seconds": 0.01}
                    )
                }
            )
        }
    )
    monkeypatch.setattr(boundedlease.gate_config, "load", lambda _root: impatient)
    monkeypatch.setattr(run_bounded_command, "_repository_root", lambda: ROOT)
    # A gate run exports its marker; setenv alone would leave it set.
    monkeypatch.delenv(LOCK.run_marker, raising=False)
    for name, value in _environment(tmp_path).items():
        monkeypatch.setenv(name, value)
    with _gate_holds_the_machine(tmp_path):
        code = run_bounded_command.run(["--timeout-seconds", "30", "--", "cargo", "build"])
    assert code == LEASE.wait_exit_code
    assert code != run_bounded_command.TIMEOUT_EXIT
