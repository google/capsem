from __future__ import annotations

import re
from pathlib import Path

import pytest
import tomllib
import variables
from capsem_builder.gate.tools.audit import public_surface as checker

ROOT = Path(__file__).resolve().parents[1]
def test_public_surfaces_match_the_approved_exact_allowlists() -> None:
    checker.check_policy()


def test_fast_feedback_is_explicitly_incomplete_and_release_owns_qualification() -> None:
    """`fast-test` is feedback; release rails own qualification.

    Reimplemented from a version that asserted on `smoke`, a single recipe
    which ran *both* -- so its name undersold the gate and oversold the loop,
    and neither half could be reasoned about on its own. The property being
    protected is unchanged: a developer-facing recipe must never become the
    thing a release lane leans on.
    """
    public_just = set(checker.current_surfaces()["just"])
    policy = tomllib.loads((ROOT / "config" / "public-surface.toml").read_text(encoding="utf-8"))[
        "just"
    ]
    for recipe in (variables.FAST_TEST, variables.FOCUS_TEST):
        assert recipe in public_just, f"{recipe} is not a public recipe"
        assert recipe in policy["approved"], f"{recipe} is not approved"
    assert "test" not in public_just
    assert "test-clean" in public_just
    assert "vm-smoke" not in public_just
    assert "smoke" not in public_just, (
        "the old bundled recipe is back; it named neither of the two jobs it ran"
    )

    # Release CI owns qualification. The local dispatcher does not require a
    # machine-specific journal from the developer feedback command.
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from capsem_builder.gate.sourcecommit import SourceCommit
    from helpers.gate import RecordingRunner

    for name, extra in (
        ("release-binaries", {"channel": "stable"}),
        ("release-profile", {"channel": "stable", "profile": "code"}),
    ):
        plan = GateCommand.registry[name](
            RecordingRunner(ROOT),
            argparse.Namespace(
                dry_run=False,
                graph=False,
                timing=False,
                source_commit=SourceCommit("0" * 40),
                **extra,
            ),
        )._describe()
        order = list(plan.labels)

        assert order[0] == "source.worktree-clean"
        assert "qualification.accept" not in order
        assert order.index("source.publish-ref") < order.index("release")
        assert not [
            label
            for label in order
            if label.startswith(("fast.", "static.", "artifacts.", "functional.", "glowup."))
        ], f"{name} repeats work already proven by the exact qualification journal"
        # By step label, not by substring: `smoke` appears in the name of a
        # test file the contracts step collects, and matching that would make
        # this pass or fail on an unrelated rename.
        assert not [label for label in order if label.startswith("smoke")], (
            f"{name} contains smoke steps, which are developer feedback and "
            "never release qualification"
        )


def test_surface_extractors_do_not_silently_return_empty_sets() -> None:
    surfaces = checker.current_surfaces()

    assert set(surfaces) == {"just", "capsem_cli", "http"}
    assert all(values for values in surfaces.values())
    assert all(values == sorted(set(values)) for values in surfaces.values())


def test_declared_count_drift_fails_closed(tmp_path: Path) -> None:
    policy = (ROOT / "config" / "public-surface.toml").read_text()

    # Derived, not hardcoded. This read `count = 13` -> `count = 14`, and the
    # day the surface legitimately grew to 14 the mutation became a no-op that
    # rewrote the file to what it already said -- a guard that passes because
    # it stopped changing anything.
    declared = tomllib.loads(policy)["just"]["count"]
    broken = tmp_path / "public-surface.toml"
    broken.write_text(
        policy.replace(f"[just]\ncount = {declared}", f"[just]\ncount = {declared + 1}")
    )

    with pytest.raises(checker.SurfaceError, match=f"policy count={declared + 1}"):
        checker.check_policy(broken)


def test_rejects_unapproved_allowlist_entry(tmp_path: Path) -> None:
    policy = (ROOT / "config" / "public-surface.toml").read_text()
    broken = tmp_path / "public-surface.toml"
    broken.write_text(
        policy.replace(
            '  "build",',
            '  "build",\n  "unapproved-command",',
            1,
        ).replace("[just]\ncount = 13", "[just]\ncount = 14")
    )

    with pytest.raises(checker.SurfaceError, match=r"missing=.*unapproved-command"):
        checker.check_policy(broken)


def test_project_skills_do_not_teach_retired_public_just_commands() -> None:
    retired = {
        "audit",
        # `bench` is not here: it was retired when nothing ran benchmarks, and
        # is approved again now that `just bench` exists. A verb can come
        # back, and `test_no_verb_is_both_retired_and_approved` is what stops
        # the two lists disagreeing about it.
        "benchmark",
        "benchmark-compare",
        "build-host-image",
        "build-kernel",
        "build-rootfs",
        "build-ui",
        "clean",
        "coverage",
        "cross-compile",
        "dev-frontend",
        "dev-tui",
        "docs",
        "inspect-session",
        "list-sessions",
        "prepare-release",
        "query-session",
        "release",
        "run-ui",
        "sandbox-logs",
        "test-artifacts",
        "test-assets",
        "test-frontend",
        "test-gateway",
        "test-gateway-e2e",
        "test-host-package-sbom",
        "test-install",
        "ui",
        "update-deps",
        "update-fixture",
        "update-prices",
    }
    command = re.compile(r"\bjust\s+([a-z][a-z0-9-]*)\b")
    failures: list[str] = []
    for path in sorted((ROOT / "skills").rglob("*.md")):
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            for match in command.finditer(line):
                if match.group(1) in retired:
                    failures.append(f"{path.relative_to(ROOT)}:{line_number}: {line.strip()}")

    assert not failures, (
        "project skills teach retired public Just commands; use the approved "
        "surface or the explicitly private owning primitive:\n" + "\n".join(failures)
    )


def test_no_verb_is_both_retired_and_approved() -> None:
    """A verb cannot be on the approved surface and taught as retired.

    `bench` was retired when nothing ran benchmarks. Reintroducing it as a
    public verb left it in both lists, so the skill documenting it failed the
    guard against teaching retired commands -- the guard was right that the
    lists disagreed and wrong about which one to believe.
    """
    source = Path(__file__).read_text(encoding="utf-8")
    block = source[source.index("    retired = {") :]
    retired = set(
        re.findall(
            r'^\s+"([a-z][a-z0-9-]*)",',
            block[: block.index("\n    }")],
            re.MULTILINE,
        )
    )
    approved = set(
        tomllib.loads((ROOT / "config" / "public-surface.toml").read_text(encoding="utf-8"))[
            "just"
        ]["approved"]
    )
    overlap = sorted(retired & approved)
    assert not overlap, (
        "these verbs are on the approved surface and also listed as retired, "
        f"so a skill cannot mention them without failing a guard: {overlap}"
    )
