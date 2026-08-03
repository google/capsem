"""What the gate may occupy, and what it must never delete.

A run builds two architectures of VM images, a package cohort, a release
channel and an install container's assets. Nothing bounded that: a crashed run
reclaimed nothing, and the next started with less room.

Most of these tests are about the second half of that sentence. Reclaiming is
whole-tree removal driven by a list in a config file, which is one editing
mistake away from being aimed somewhere terrible -- so the guards matter more
than the feature.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.disk import ensure_space, footprint, reclaim, roots
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SOURCE = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")


def _checkout(tmp_path: Path, **overrides: object) -> gate_config.GateConfig:
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    source = SOURCE
    for key, value in overrides.items():
        original = next(
            line for line in source.splitlines() if line.startswith(f"{key} = ")
        )
        source = source.replace(original, f"{key} = {value}")
    (tmp_path / "config" / "gate.toml").write_text(source, encoding="utf-8")
    return gate_config.load(tmp_path)


def _occupy(config: gate_config.GateConfig, relative: str, *, size: int = 1024) -> Path:
    target = config.path(relative)
    (target / "nested").mkdir(parents=True)
    (target / "nested" / "blob").write_bytes(b"x" * size)
    return target


# ---------------------------------------------------------------------------
# Reclaiming
# ---------------------------------------------------------------------------


def test_reclaiming_removes_the_declared_trees(tmp_path: Path) -> None:
    config = _checkout(tmp_path)
    built = _occupy(config, "target/test-home")

    recovered = reclaim(config)

    assert not built.exists()
    assert recovered.bytes_freed > 0


def test_reclaiming_reports_what_each_tree_gave_back(tmp_path: Path) -> None:
    """So `gc --dry-run` can say where the space actually is."""
    config = _checkout(tmp_path)
    _occupy(config, "target/test-home", size=4096)
    _occupy(config, "target/image-workspace", size=1024)

    recovered = reclaim(config)

    assert recovered.trees["target/test-home"] > recovered.trees["target/image-workspace"]


def test_a_tree_the_run_still_needs_is_kept(tmp_path: Path) -> None:
    """Reclaiming during a run must not take the run's own log with it."""
    config = _checkout(tmp_path)
    _occupy(config, "target/gate-runs")
    _occupy(config, "target/test-home")

    reclaim(config, keep=("target/gate-runs",))

    assert config.path("target/gate-runs").exists()
    assert not config.path("target/test-home").exists()


def test_reclaiming_what_is_not_there_is_harmless(tmp_path: Path) -> None:
    """Teardown runs against whatever state a failure left, often nothing."""
    config = _checkout(tmp_path)

    assert reclaim(config).bytes_freed == 0


# ---------------------------------------------------------------------------
# What must never be deleted
# ---------------------------------------------------------------------------


def test_a_symlink_inside_a_reclaimed_tree_is_unlinked_not_followed(
    tmp_path: Path,
) -> None:
    """The failure mode that would make this feature not worth having.

    Somebody's `target/test-home/assets` pointing at a real asset tree, or a
    developer's link into their home directory, must cost them the link and
    nothing else.
    """
    config = _checkout(tmp_path)
    precious = tmp_path / "precious"
    precious.mkdir()
    (precious / "irreplaceable").write_text("years of work")

    home = config.path("target/test-home")
    home.mkdir(parents=True)
    (home / "shortcut").symlink_to(precious)

    reclaim(config)

    assert not home.exists()
    assert (precious / "irreplaceable").read_text() == "years of work"


def test_a_reclaimable_root_that_is_itself_a_symlink_is_only_unlinked(
    tmp_path: Path,
) -> None:
    """Someone points `target/test-home` at a scratch volume; reclaiming it
    should return the link, not empty the volume."""
    config = _checkout(tmp_path)
    volume = tmp_path / "scratch-volume"
    (volume / "data").mkdir(parents=True)
    (volume / "data" / "blob").write_bytes(b"x" * 512)

    link = config.path("target/test-home")
    link.parent.mkdir(parents=True, exist_ok=True)
    link.symlink_to(volume)

    reclaim(config)

    assert not link.exists()
    assert (volume / "data" / "blob").exists()


def test_a_tree_resolving_outside_the_checkout_is_refused(tmp_path: Path) -> None:
    """The resolved-path check behind the loader's string check.

    Config validation rejects an absolute or upward-escaping entry; this
    catches the case where the string is innocent and the filesystem is not.
    """
    from capsem.gate.disk import _remove_tree

    outside = tmp_path / "outside"
    outside.mkdir()

    with pytest.raises(GateError, match="outside"):
        _remove_tree(outside, tmp_path / "checkout")


def test_the_declared_roots_all_sit_under_the_checkout() -> None:
    """Read against the real configuration, not a fixture."""
    config = gate_config.load(PROJECT_ROOT)

    for path in roots(config):
        assert PROJECT_ROOT in path.parents


# ---------------------------------------------------------------------------
# Making room
# ---------------------------------------------------------------------------


def test_a_run_with_room_reclaims_nothing(tmp_path: Path) -> None:
    """Deleting caches nobody asked about, on a machine with space, is how a
    gate earns a reputation for being slow."""
    config = _checkout(tmp_path, required_free_gb=0)
    built = _occupy(config, "target/test-home")

    ensure_space(config, "assets")

    assert built.exists()


def test_running_short_reclaims_before_refusing(tmp_path: Path) -> None:
    """Failing having deleted nothing is the worst of both."""
    config = _checkout(tmp_path, required_free_gb=10**9)
    built = _occupy(config, "target/test-home")

    with pytest.raises(GateError):
        ensure_space(config, "assets")

    assert not built.exists(), "it must try before it gives up"


def test_reclaiming_enough_lets_the_phase_proceed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The path that makes reclaiming worth attempting at all.

    Real free space cannot be moved by a test, so the measurement is stubbed:
    tight before the reclaim, comfortable after.
    """
    config = _checkout(tmp_path, required_free_gb=50)
    built = _occupy(config, "target/test-home")
    readings = iter([10.0, 10.0, 80.0, 80.0, 80.0])
    monkeypatch.setattr("capsem.gate.disk.free_gb", lambda _root: next(readings))

    recovered = ensure_space(config, "assets")

    assert not built.exists()
    assert recovered.free_after_gb >= 50


def test_refusing_says_what_is_needed_and_what_is_left(tmp_path: Path) -> None:
    """A number and a next step, rather than 'no space left on device' from
    somewhere inside a Docker build an hour later."""
    config = _checkout(tmp_path, required_free_gb=10**9)

    with pytest.raises(GateError) as failure:
        ensure_space(config, "assets")

    message = str(failure.value)
    assert "assets" in message
    assert "gc" in message, "and where to go next"


def test_the_run_log_survives_making_room(tmp_path: Path) -> None:
    """The record of the run doing the reclaiming is not spare capacity."""
    config = _checkout(tmp_path, required_free_gb=10**9)
    _occupy(config, "target/gate-runs")

    with pytest.raises(GateError):
        ensure_space(config, "assets")

    assert config.path("target/gate-runs").exists()


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def test_footprint_measures_each_tree_separately(tmp_path: Path) -> None:
    """`gc --dry-run` has to say where the space is, not just how much."""
    config = _checkout(tmp_path)
    _occupy(config, "target/test-home", size=2048)

    measured = footprint(config)

    assert measured["target/test-home"] >= 2048
    assert "target/image-workspace" not in measured, "absent trees are not zero rows"


def test_a_reclaim_does_not_delete_the_run_it_is_writing(tmp_path: Path) -> None:
    """`gc` reclaims `target/gate-runs`, and records into it.

    Those were compatible only while `gc` recorded nothing. Making a real
    reclaim leave durable evidence -- which it should, since it deletes whole
    trees -- put a live run log inside the very tree the step removes, and the
    next journal write failed with `FileNotFoundError`. The run log is bounded
    by its own retention policy; the blunt reclaimer has no business in it
    while a run is open.

    `ensure_space` already passed `keep=(runlog.root,)`. This is the caller
    that did not.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.context import Context
    from capsem.gate.gc import _trees
    from capsem.gate.runlog import RunLog

    from helpers.gate import RecordingRunner

    (tmp_path / "config").mkdir(parents=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"), encoding="utf-8"
    )
    config = gate_config.load(tmp_path)

    # Something reclaimable to remove, so the step does real work.
    reclaimable = config.path(config.disk.reclaimable[0])
    if reclaimable != config.path(config.runlog.root):
        reclaimable.mkdir(parents=True, exist_ok=True)
        (reclaimable / "junk").write_text("x", encoding="utf-8")

    with RunLog.open(config, "gc") as log:
        directory = log.directory
        _trees(Context(RecordingRunner(tmp_path), config, journal=log))
        # The write that used to raise.
        log.note("still here")

    assert directory.is_dir(), "the reclaim deleted the run it was writing"
    assert (directory / config.runlog.summary).is_file()
