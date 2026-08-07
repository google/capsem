from __future__ import annotations

import importlib.util
import re
import tomllib
from pathlib import Path

import pytest
import variables

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check_public_surface.py"


def _load_checker():
    spec = importlib.util.spec_from_file_location("check_public_surface", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_public_surfaces_match_the_approved_exact_allowlists() -> None:
    _load_checker().check_policy()


def test_the_fast_gate_is_public_and_is_not_a_release_shortcut() -> None:
    """`fast-test` is the fast gate; `vm-smoke` is a runtime liveness check.

    Reimplemented from a version that asserted on `smoke`, a single recipe
    which ran *both* -- so its name undersold the gate and oversold the loop,
    and neither half could be reasoned about on its own. The property being
    protected is unchanged: a developer-facing recipe must never become the
    thing a release lane leans on.
    """
    checker = _load_checker()
    public_just = set(checker.current_surfaces()["just"])
    policy = tomllib.loads(
        (ROOT / "config" / "public-surface.toml").read_text(encoding="utf-8")
    )["just"]
    assert "test" in public_just
    for recipe in (variables.FAST_TEST, variables.VM_SMOKE):
        assert recipe in public_just, f"{recipe} is not a public recipe"
        assert recipe in policy["approved"], f"{recipe} is not approved"
    assert "smoke" not in public_just, (
        "the old bundled recipe is back; it named neither of the two jobs it ran"
    )

    # Each release command runs the complete gate before it publishes, and
    # never the reduced developer feedback. The evidence moved from `just test`
    # appearing in the recipe to the gate's own phases appearing in the plan:
    # a recipe line proved a command was named, where this proves the work is
    # actually there and sits ahead of every publishing step.
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    for name, extra in (
        ("release-binaries", {"channel": "nightly"}),
        ("release-profile", {"channel": "nightly", "profile": "code"}),
    ):
        plan = GateCommand.registry[name](
            RecordingRunner(ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False, **extra),
        )._describe()
        order = list(plan.labels)

        phases = [
            next(i for i, label in enumerate(order) if label.startswith(prefix))
            for prefix in ("fast.", "static.", "artifacts.", "functional.", "glowup.")
        ]
        assert max(phases) < order.index("release"), (
            f"{name} publishes before the complete gate has passed"
        )
        # By step label, not by substring: `smoke` appears in the name of a
        # test file the contracts step collects, and matching that would make
        # this pass or fail on an unrelated rename.
        assert not [label for label in order if label.startswith("smoke")], (
            f"{name} contains smoke steps, which are developer feedback and "
            "never release qualification"
        )


def test_surface_extractors_do_not_silently_return_empty_sets() -> None:
    checker = _load_checker()
    surfaces = checker.current_surfaces()

    assert set(surfaces) == {"just", "capsem_cli", "http"}
    assert all(values for values in surfaces.values())
    assert all(values == sorted(set(values)) for values in surfaces.values())


def test_declared_count_drift_fails_closed(tmp_path: Path) -> None:
    checker = _load_checker()
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
    checker = _load_checker()
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
        "bench",
        "benchmark",
        "benchmark-compare",
        "build-assets",
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
        "install",
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
        "test-linux-rust",
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
                    failures.append(
                        f"{path.relative_to(ROOT)}:{line_number}: {line.strip()}"
                    )

    assert not failures, (
        "project skills teach retired public Just commands; use the approved "
        "surface or the explicitly private owning primitive:\n" + "\n".join(failures)
    )
