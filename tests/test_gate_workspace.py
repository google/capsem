"""The isolated home, and the order that made three recipes get it wrong.

`_test-candidate-run`, `smoke` and the asset gate each hand-wrote this: an
isolated `CAPSEM_HOME`, the same four exported variables, and an EXIT trap to
stop whatever service ended up in it. Each got slightly different details
right, and the details are the whole thing.

Two orderings in particular are load-bearing and neither is visible in a
`finally` block. The service must stop before the run directory is removed,
because stopping it is what flushes `serial.log` -- the file a boot failure is
argued from. And evidence must be copied out before either, because both
destroy it.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError
from capsem.gate.lifecycle import held
from capsem.gate.workspace import Workspace

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SOURCE = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")


def _checkout(tmp_path: Path) -> gate_config.GateConfig:
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    (tmp_path / "config" / "gate.toml").write_text(SOURCE, encoding="utf-8")
    return gate_config.load(tmp_path)


# ---------------------------------------------------------------------------
# Isolation
# ---------------------------------------------------------------------------


def test_the_workspace_is_wiped_on_the_way_in(tmp_path: Path) -> None:
    """On entry rather than exit: a crashed run leaves its home for
    inspection, and the next run is the one that no longer needs it."""
    config = _checkout(tmp_path)
    workspace = Workspace(config)
    workspace.home.mkdir(parents=True)
    (workspace.home / "from-a-previous-run").write_text("stale")

    workspace.acquire()

    assert not (workspace.home / "from-a-previous-run").exists()
    assert workspace.run_dir.is_dir()


def test_the_directories_the_service_writes_to_exist_up_front(
    tmp_path: Path,
) -> None:
    """Creating them lazily was how a first run differed from every later one."""
    config = _checkout(tmp_path)
    workspace = Workspace(config)

    workspace.acquire()

    for relative in config.workspace.seeded_dirs:
        assert (workspace.home / relative).is_dir()


def test_the_environment_points_every_command_at_this_home(tmp_path: Path) -> None:
    """Exported once, rather than by each invocation remembering -- which is
    how one of them stopped remembering and wrote into the developer's own
    `~/.capsem`."""
    config = _checkout(tmp_path)

    environment = Workspace(config).environment()

    assert environment["CAPSEM_HOME"].endswith(config.workspace.home)
    assert environment["CAPSEM_RUN_DIR"].startswith("/tmp/capsem-r-")
    assert not environment["CAPSEM_RUN_DIR"].startswith(environment["CAPSEM_HOME"])
    assert "CAPSEM_BENCHMARK_OUTPUT_ROOT" in environment
    assert "COVERAGE_FILE" in environment


def test_the_benchmark_recordings_survive_the_workspace(tmp_path: Path) -> None:
    """`just test` runs several modules through one workspace and the VM
    recordings come from the functional one. A later module clearing them is
    why a fortnight of full gates left this empty and froze the published
    arm64 history."""
    config = _checkout(tmp_path)
    recordings = config.path(config.workspace.benchmark_root)
    recordings.mkdir(parents=True)
    (recordings / "arm64.json").write_text("{}")

    workspace = Workspace(config)
    workspace.acquire()
    workspace.release()

    assert (recordings / "arm64.json").exists()


# ---------------------------------------------------------------------------
# Giving it back
# ---------------------------------------------------------------------------


def test_the_service_is_stopped_before_the_run_directory_goes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Stopping it is what flushes serial.log, and the run directory is where
    that lands. Deleting first loses the only evidence a boot failure has."""
    config = _checkout(tmp_path)
    workspace = Workspace(config)
    workspace.acquire()
    order: list[str] = []

    def stopped(run_dir, settings):
        order.append(f"stop {run_dir.is_dir()}")

    monkeypatch.setattr("capsem.gate.pidfiles.stop_gate_service", stopped)

    workspace.release()

    assert order == ["stop True"], "the run directory must still exist at stop time"
    assert not workspace.run_dir.exists()


def test_evidence_is_copied_out_before_anything_removes_it(tmp_path: Path) -> None:
    config = _checkout(tmp_path)
    workspace = Workspace(config)
    workspace.acquire()
    (workspace.run_dir / "vm").mkdir(parents=True)
    (workspace.run_dir / "vm" / "serial.log").write_text("kernel panic")

    workspace.preserve(GateError("boot failed"))
    workspace.release()

    assert workspace.preserved is not None
    assert (workspace.preserved / "vm" / "serial.log").read_text() == "kernel panic"
    assert not workspace.run_dir.exists()


def test_evidence_collection_skips_what_is_not_a_log(tmp_path: Path) -> None:
    """A run directory holds rootfs overlays; copying those out fills a disk
    the failure already strained."""
    config = _checkout(tmp_path)
    workspace = Workspace(config)
    workspace.acquire()
    (workspace.run_dir / "vm").mkdir(parents=True)
    (workspace.run_dir / "vm" / "serial.log").write_text("boot")
    (workspace.run_dir / "vm" / "rootfs.img").write_bytes(b"x" * 4096)

    workspace.preserve(GateError("boot failed"))

    assert workspace.preserved is not None
    assert (workspace.preserved / "vm" / "serial.log").exists()
    assert not (workspace.preserved / "vm" / "rootfs.img").exists()


def test_a_passing_run_preserves_nothing(tmp_path: Path) -> None:
    config = _checkout(tmp_path)
    workspace = Workspace(config)

    with held(workspace):
        pass

    assert workspace.preserved is None


def test_a_failing_run_preserves_and_still_releases(tmp_path: Path) -> None:
    """`held` runs preserve before release, so this is the shape rather than
    an ordering a `finally` block happens to have."""
    config = _checkout(tmp_path)
    workspace = Workspace(config)

    with pytest.raises(GateError, match="boot failed"), held(workspace):
        (workspace.run_dir / "vm").mkdir(parents=True)
        (workspace.run_dir / "vm" / "serial.log").write_text("panic")
        raise GateError("boot failed")

    assert workspace.preserved is not None
    assert (workspace.preserved / "vm" / "serial.log").read_text() == "panic"
    assert not workspace.run_dir.exists()


def test_releasing_a_workspace_that_was_never_built_is_harmless(
    tmp_path: Path,
) -> None:
    """Teardown runs against whatever state a failure left behind."""
    Workspace(_checkout(tmp_path)).release()


# ---------------------------------------------------------------------------
# Where it sits
# ---------------------------------------------------------------------------


def test_the_home_is_reclaimable_and_the_lock_is_not_inside_it() -> None:
    """The run takes the machine lock and then wipes this. A lockfile inside
    would be unlinked while held, and the next run would lock a fresh inode."""
    config = gate_config.load(PROJECT_ROOT)

    assert any(config.workspace.home.startswith(entry) for entry in config.disk.reclaimable), (
        "the workspace must be reclaimable by `gc`"
    )
    assert not Path(config.locks.gate.path).is_relative_to(Path(config.workspace.home))
