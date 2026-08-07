"""The Linux builder image is a step, and the two lanes that need it share it.

`installimage.prepare()` and `CrossCompiler._prepare_builder()` both ran
`just _build-host-image`. That recipe does not exist -- the justfile carries its
heading and no body, and `just --show _build-host-image` has been failing for as
long as the calls have been there. Install-image preflight and cross-compilation
were both broken at runtime, which is to say static qualification and the
package lanes were.

Nothing noticed because nothing checked that a name written in Python resolves
to something real, and because the unit tests around both modules stopped at
the recipe boundary rather than crossing it.

The image is `hostimage.image(config)`, a step that already existed. Composed
rather than dispatched, it is built once per plan however many lanes want it.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import (
    cli,  # noqa: F401 - imported so every command registers
    hostimage,
)
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand
from capsem.gate.context import Context
from capsem.gate.dockermount import Mount
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _plan(name: str, **args) -> Plan:
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )._describe()


@pytest.mark.parametrize(
    ("name", "args"),
    [("install-image", {}), ("cross-compile", {"arch": "arm64"})],
)
def test_the_builder_image_is_a_step_rather_than_a_recipe(name, args) -> None:
    """Not `just _build-host-image`, which has never existed."""
    plan = _plan(name, **args)

    assert hostimage.STEP in plan.labels, (
        f"{name} does not build the host image it depends on: {plan.labels}"
    )


@pytest.mark.parametrize(
    ("name", "args"),
    [("install-image", {}), ("cross-compile", {"arch": "arm64"})],
)
def test_everything_that_needs_the_builder_waits_for_it(name, args) -> None:
    """A lane that builds a package before its builder exists fails late."""
    plan = _plan(name, **args)
    order = list(plan.labels)
    built = order.index(hostimage.STEP)

    assert built == 0 or all(
        order.index(label) > built
        for label in order
        if label.startswith(("install-image", "package."))
    )


def test_two_lanes_in_one_plan_build_the_builder_once() -> None:
    """The diamond `shared` exists for.

    Composed into a candidate plan these lanes both want the image; building a
    six-gigabyte Docker image twice is the waste, and adding it twice is a
    duplicate-label error that would stop the composition outright.
    """
    plan = Plan("composed")
    hostimage.fragment(plan, CONFIG)
    hostimage.fragment(plan, CONFIG)

    assert list(plan.labels).count(hostimage.STEP) == 1


def test_foreign_uid_probe_mounts_linked_worktree_metadata(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    metadata = "/git/common"
    monkeypatch.setattr(
        hostimage,
        "docker_git_metadata_mount",
        lambda _runner: Mount.unmigrated(metadata, metadata, "ro"),
    )
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies={"git rev-parse --short HEAD": "abc123"},
    )

    hostimage._ForeignUidProbe().perform(Context(runner, CONFIG))

    probe = runner.rendered[-1]
    assert f"-v {metadata}:{metadata}:ro" in probe
    assert "--user 4242:4242" in probe


def test_only_the_lane_that_needs_git_provenance_still_carries_it() -> None:
    """Reimplemented, not deleted. The claim survives; its subject moved.

    This asserted that the parity lane mounted a linked worktree's git
    metadata, ran as the host user, and grafted a writable directory for
    Tauri's generated ACLs through the read-only source mount. All three were
    properties of the bind mount, and the lane has none.

    What must not be lost is *why* the metadata mount existed: `build.rs`
    embeds `git rev-parse --short HEAD` and falls back to the string `unknown`
    rather than failing, so a container that cannot read git history produces a
    binary with no source identity -- silently. That matters exactly where a
    shipped artifact is built, and not at all where coverage is measured.

    So the claim is now a distinction, asserted in both directions: the package
    rail, which builds the artifact a release publishes, still carries git
    metadata into its container; the parity lane, which only measures which
    Linux branches execute, deliberately does not and takes the `unknown`
    fallback. Asserting only the first half would let the mount quietly come
    back; asserting only the second would let provenance quietly leave.
    """
    from capsem.gate import linuxrust

    rail = (PROJECT_ROOT / "src/capsem/gate/packagerail.py").read_text(encoding="utf-8")
    assert "docker_git_metadata_mount" in rail, (
        "the package rail stopped carrying git metadata, so a published binary "
        "would embed an 'unknown' build hash without anything failing"
    )

    lane = (PROJECT_ROOT / "src/capsem/gate/linuxrust.py").read_text(encoding="utf-8")
    assert "docker_git_metadata_mount" not in lane
    for flag in ("-v", "--volume", "--user"):
        assert flag not in lane, f"the parity lane grew a {flag}, so it shares state again"

    # And the probe that proves the builder can read a checkout it does not own
    # is still wired, because that is what makes the package rail's mount
    # sufficient rather than merely present.
    assert "_ForeignUidProbe" in (PROJECT_ROOT / "src/capsem/gate/hostimage.py").read_text(
        encoding="utf-8"
    )
    assert linuxrust.RunLane is not None


def test_the_lane_has_somewhere_to_put_its_output_before_it_runs() -> None:
    """Reimplemented, not deleted. The claim survives; its mechanism does not.

    This asserted that nested mountpoints were materialized before the
    read-only source mount, because grafting a writable path through a `:ro`
    mount fails if the directory underneath does not already exist. There are
    no mounts now -- the lane copies its source into an image and copies its
    coverage back out -- so the equivalent claim is that the destination
    exists before `docker cp` writes into it, and that the copy happens on the
    failure path too, since a lane that failed is exactly when its coverage is
    worth having.
    """
    from capsem.gate import linuxrust

    source = (PROJECT_ROOT / "src/capsem/gate/linuxrust.py").read_text(encoding="utf-8")
    body = source[source.index("def perform") :]

    assert body.index("make_dir(destination)") < body.index("copy_out("), (
        "the coverage destination is created after the copy that writes to it"
    )
    assert "finally:" in body and body.index("finally:") < body.index("copy_out("), (
        "the extraction is not on the failure path, so a failed lane loses the "
        "coverage that explains it"
    )
    assert body.index("copy_out(") < body.rindex("remove("), (
        "the container is removed before its output is copied out"
    )
    assert linuxrust.RunLane().render()


def test_chained_lanes_do_not_make_the_builder_depend_on_them() -> None:
    """Shared groundwork sits before everything, not after its caller.

    The glow-up lane chains architectures so the second package build waits for
    the first to release its disk. Passing that `after` down to the shared
    host-image step made the image depend on a package that depends on the
    image -- a cycle, and one that only appears once two lanes are composed
    into a single plan.

    The builder image has no ordering requirement of its own. Only the package
    that runs inside it does.
    """
    from capsem.gate import crosscompile

    plan = Plan("chained")
    first = crosscompile.fragment(plan, CONFIG, CONFIG.arch("arm64"))
    crosscompile.fragment(plan, CONFIG, CONFIG.arch("x86_64"), after=(first,))

    assert plan.labels, "a cycle would raise before returning any order"
    assert list(plan.labels).count(hostimage.STEP) == 1
