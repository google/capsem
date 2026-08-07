"""Docker builds retain Git identity without a repository in the container.

Reimplemented, not deleted. The original asserted that a linked worktree's
common `.git` directory was bind-mounted at its absolute path so a container
could run `git rev-parse` and `build.rs` would not embed `"unknown"`. That
whole mechanism is gone: no lane mounts the checkout any more, `.dockerignore`
excludes `.git`, and shipping a repository into every lane image to answer one
question was the wrong trade.

The property it protected is not gone, so neither is the test. A package built
by the gate must still carry the exact revision of the tree it was built from,
whether or not that tree is a linked worktree. What changed is how: the gate
passes the revision it already recorded, instead of the container discovering
it from a repository that may or may not be reachable.

This is the difference between testing a mechanism and testing an outcome. The
mechanism was replaced; the outcome is the same and is asserted here.
"""

from __future__ import annotations

from pathlib import Path

from capsem.gate import config as gate_config
from capsem.gate.packageinputs import package_environment

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _target():
    return CONFIG.arch(next(iter(CONFIG.architectures)))


def test_the_revision_reaches_the_builder_as_a_declared_input() -> None:
    """Told, not discovered.

    `check-build-provenance.sh` refuses a binary that does not embed the
    expected revision, so a container that cannot determine one fails the
    build rather than shipping a mislabelled package. Under the old bind
    mount it determined one by reading a repository; there is none now.
    """
    environment = package_environment(
        CONFIG,
        _target(),
        toolchain="1.97.1",
        manifest_url="file:///src/assets/local/manifest.json",
        signing={},
        revision="abc1234",
    )

    assert environment[CONFIG.environment.package.build_revision] == "abc1234"


def test_a_worktree_needs_no_special_handling_now() -> None:
    """The linked-worktree case is not a case any more.

    A linked worktree's `.git` is a *file* pointing into another repository,
    which is exactly why it needed a second mount at an absolute host path.
    Nothing reads a repository inside a container, so the distinction between
    an ordinary checkout and a linked worktree stops existing at this boundary
    -- and `snapshot._require_own_repository` refuses to build a prefix from a
    linked worktree anyway, which is a louder and earlier answer.
    """
    from capsem.gate import snapshot

    assert hasattr(snapshot, "_require_own_repository")
    assert not (PROJECT_ROOT / "src" / "capsem" / "gate" / "gitmetadata.py").exists(), (
        "gitmetadata is back; if a lane needs Git identity again it should be "
        "passed in, not mounted"
    )


def test_no_lane_mounts_git_metadata() -> None:
    """The ratchet: this is the shape that must not come back.

    A mount of the host's Git directory is a mount of the checkout by another
    name -- it shares inodes with every host step in exactly the way that
    killed a release run.
    """
    gate = PROJECT_ROOT / "src" / "capsem" / "gate"
    offenders = [
        path.name
        for path in gate.glob("*.py")
        if "git-common-dir" in path.read_text(encoding="utf-8")
    ]
    assert not offenders, f"these resolve a Git directory to mount it: {offenders}"
