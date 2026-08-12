from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "release-binaries.py"
SPEC = importlib.util.spec_from_file_location("release_binaries", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
RELEASE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RELEASE
SPEC.loader.exec_module(RELEASE)
NOTES_SCRIPT = PROJECT_ROOT / "scripts" / "extract-release-notes.py"
NOTES_SPEC = importlib.util.spec_from_file_location("extract_release_notes", NOTES_SCRIPT)
assert NOTES_SPEC is not None and NOTES_SPEC.loader is not None
NOTES = importlib.util.module_from_spec(NOTES_SPEC)
NOTES_SPEC.loader.exec_module(NOTES)

# Fixture versions follow the release line the script enforces rather than
# restating it. Hardcoded fixtures failed this whole suite when the line moved,
# reporting an inconsistent cohort instead of a stale test.
_LINE_MAJOR, _LINE_MINOR = (int(part) for part in RELEASE.RELEASE_LINE.split("."))
COHORT_VERSION = f"{RELEASE.RELEASE_LINE}.2"
OLDER_VERSION = f"{RELEASE.RELEASE_LINE}.1"
# Deliberately off the release line, to prove a mixed cohort is rejected.
OFF_LINE_VERSION = f"{_LINE_MAJOR}.{_LINE_MINOR - 1}.1"


class FakeRunner:
    def __init__(
        self,
        root: Path,
        *,
        unexpected: bool = False,
        current_release_tag: str = "",
        omit_cargo_lock_change: bool = False,
        omit_release_notes_change: bool = False,
        mixed_version_cohort: bool = False,
        current_release_channel: str | None = None,
        existing_version_tag: str = "",
        run_rows: list[dict[str, object]] | None = None,
        pending_local_release: bool = False,
        divergent_pending_release: bool = False,
        fail_commit: bool = False,
        fail_tag: bool = False,
    ) -> None:
        self.root = root
        self.unexpected = unexpected
        self.current_release_tag = current_release_tag
        self.omit_cargo_lock_change = omit_cargo_lock_change
        self.omit_release_notes_change = omit_release_notes_change
        self.mixed_version_cohort = mixed_version_cohort
        self.current_release_channel = current_release_channel
        self.existing_version_tag = existing_version_tag
        self.run_rows = run_rows
        self.pending_local_release = pending_local_release
        self.divergent_pending_release = divergent_pending_release
        self.fail_commit = fail_commit
        self.fail_tag = fail_tag
        self.calls: list[tuple[str, ...]] = []
        self.stamped = False
        self.dispatched_tag = ""
        self.dispatched_channel = ""
        self.dispatched_id = ""
        self.dispatched_publish = ""
        self.dispatched_ref = ""
        self.head = "a" * 40

    def run(
        self,
        argv: Sequence[str],
        *,
        capture: bool = False,
        env: dict[str, str] | None = None,
    ):
        del capture, env
        command = tuple(argv)
        self.calls.append(command)
        if command == ("git", "status", "--porcelain", "--untracked-files=all"):
            if not self.stamped:
                return RELEASE.CommandResult("")
            paths = list(RELEASE.MUTATED_PATHS)
            if self.omit_cargo_lock_change:
                paths.remove(Path("Cargo.lock"))
            if self.omit_release_notes_change:
                for path in RELEASE.RELEASE_NOTE_PATHS:
                    paths.remove(path)
            if self.unexpected:
                paths.append(Path("config/profiles/code/profile.toml"))
            return RELEASE.CommandResult("\n".join(f" M {path}" for path in paths))
        if command == ("git", "branch", "--show-current"):
            return RELEASE.CommandResult("main\n")
        if command in (("git", "rev-parse", "HEAD"),):
            return RELEASE.CommandResult(self.head + "\n")
        if command == ("git", "rev-parse", "origin/main"):
            value = "b" * 40 if self.pending_local_release else "a" * 40
            return RELEASE.CommandResult(value + "\n")
        if command == ("git", "rev-parse", "HEAD^"):
            value = "c" * 40 if self.divergent_pending_release else "b" * 40
            return RELEASE.CommandResult(value + "\n")
        if command == ("git", "rev-list", "--count", "origin/main..HEAD"):
            return RELEASE.CommandResult("1\n" if self.pending_local_release else "0\n")
        if command == ("git", "log", "-1", "--format=%s"):
            return RELEASE.CommandResult(
                f"release({self.current_release_channel}): {self.current_release_tag}\n"
            )
        if command == ("git", "tag", "--points-at", "HEAD", "--list", "v*"):
            return RELEASE.CommandResult(f"{self.current_release_tag}\n")
        if command == (
            "git",
            "tag",
            "--list",
            self.current_release_tag,
            "--format=%(contents)",
        ):
            suffix = (
                f" channel={self.current_release_channel}"
                if self.current_release_channel is not None
                else ""
            )
            return RELEASE.CommandResult(
                f"Capsem {self.current_release_tag.removeprefix('v')}{suffix}\n"
            )
        if command == ("just", "_stamp-version"):
            self.stamped = True
            for path in RELEASE.MUTATED_PATHS:
                target = self.root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(f"changed {path}\n", encoding="utf-8")
            (self.root / "Cargo.toml").write_text(
                f'[workspace.package]\nversion = "{COHORT_VERSION}"\n',
                encoding="utf-8",
            )
            (self.root / "Cargo.lock").write_text(
                "[[package]]\n"
                'name = "capsem"\n'
                f'version = "{OFF_LINE_VERSION if self.mixed_version_cohort else COHORT_VERSION}"\n'
                "\n[[package]]\n"
                'name = "capsem-core"\n'
                f'version = "{COHORT_VERSION}"\n',
                encoding="utf-8",
            )
            (self.root / "crates/capsem-app/tauri.conf.json").write_text(
                f'{{"version": "{COHORT_VERSION}"}}\n',
                encoding="utf-8",
            )
            (self.root / "pyproject.toml").write_text(
                f'[project]\nversion = "{COHORT_VERSION}"\n',
                encoding="utf-8",
            )
            (self.root / "uv.lock").write_text(
                f'[[package]]\nname = "capsem"\nversion = "{COHORT_VERSION}"\n',
                encoding="utf-8",
            )
            return RELEASE.CommandResult("")
        if len(command) >= 2 and command[1] == "scripts/extract-release-notes.py":
            return RELEASE.CommandResult("")
        if command[:3] == ("git", "tag", "--list"):
            if len(command) == 4 and command[3] == self.existing_version_tag:
                return RELEASE.CommandResult(f"{self.existing_version_tag}\n")
            return RELEASE.CommandResult("")
        if command[:2] == ("git", "commit"):
            if self.fail_commit:
                raise subprocess.CalledProcessError(1, command)
            self.head = "d" * 40
            return RELEASE.CommandResult("")
        if command[:2] == ("git", "tag"):
            if self.fail_tag:
                raise subprocess.CalledProcessError(1, command)
            return RELEASE.CommandResult("")
        if command[:3] == ("git", "reset", "--mixed"):
            self.head = command[3]
            return RELEASE.CommandResult("")
        if command[:3] == ("gh", "workflow", "run"):
            self.dispatched_tag = command[command.index("--ref") + 1]
            self.dispatched_ref = self.dispatched_tag
            fields = [command[index + 1] for index, value in enumerate(command) if value == "-f"]
            self.dispatched_channel = next(
                value.removeprefix("channel=") for value in fields if value.startswith("channel=")
            )
            self.dispatched_id = next(
                value.removeprefix("dispatch_id=")
                for value in fields
                if value.startswith("dispatch_id=")
            )
            self.dispatched_publish = next(
                value.removeprefix("publish=") for value in fields if value.startswith("publish=")
            )
            self.dispatched_tag = next(
                value.removeprefix("tag=") for value in fields if value.startswith("tag=")
            )
            return RELEASE.CommandResult("")
        if command[:4] == ("gh", "run", "list", "--workflow"):
            rows = list(self.run_rows or [])
            if self.dispatched_tag:
                rows.insert(
                    0,
                    {
                        "databaseId": 42,
                        "displayTitle": (
                            f"Release {self.dispatched_channel} {self.dispatched_tag} "
                            f"{self.dispatched_id}"
                        ),
                        "status": "in_progress",
                        "conclusion": "",
                    },
                )
            return RELEASE.CommandResult(json.dumps(rows))
        return RELEASE.CommandResult("")


def _release_tree(tmp_path: Path) -> None:
    for relative in RELEASE.MUTATED_PATHS:
        path = tmp_path / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"original {relative}\n", encoding="utf-8")
    (tmp_path / "Cargo.toml").write_text(
        f'[workspace.package]\nversion = "{OLDER_VERSION}"\n',
        encoding="utf-8",
    )


def _release_plan(command: str, *arguments: str):
    """The plan a release command would run, without running any of it."""
    import argparse

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand
    from capsem.gate.proc import Runner

    parsed = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        **dict(zip(("channel", "profile"), arguments, strict=False)),
    )
    return GateCommand.registry[command](Runner(PROJECT_ROOT), parsed).plan()


def test_binary_release_owns_one_scripted_build_and_dispatch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path)

    tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert tag == f"v{COHORT_VERSION}"
    assert run_id == "42"
    assert ("just", "_stamp-version") in runner.calls
    assert (
        "gh",
        "workflow",
        "run",
        "release.yaml",
        "--ref",
        tag,
        "-f",
        f"tag={tag}",
        "-f",
        "channel=nightly",
        "-f",
        "publish=true",
        "-f",
    ) == next(call[:-1] for call in runner.calls if call[:3] == ("gh", "workflow", "run"))
    assert runner.dispatched_id.startswith("capsem-binaries-")
    assert ("gh", "run", "watch", "42", "--exit-status") in runner.calls
    joined = "\n".join(" ".join(call) for call in runner.calls)
    assert "_build-kernel" not in joined
    assert "_build-rootfs" not in joined
    assert "release-assets.yaml" not in joined


def test_binary_release_checks_notes_before_version_stamp(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path)

    RELEASE.release_binaries("nightly", runner)

    check = runner.calls.index((sys.executable, "scripts/extract-release-notes.py", "--check"))
    stamp = runner.calls.index(("just", "_stamp-version"))
    assert check < stamp


def test_binary_recipe_checks_release_intent_before_complete_local_gate_and_push() -> None:
    """Missing release notes cost seconds, not a complete gate.

    Asked of the plan: the recipe dispatches, so its text no longer carries
    the order. The plan does, as edges rather than line positions.
    """
    plan = _release_plan("release-binaries", "nightly")
    order = list(plan.labels)

    assert "release-binaries.py --precheck nightly" in plan.describe()
    # Every phase of the gate sits between them: there is no step named `gate`
    # now that the release plan contains the gate rather than launching it.
    first_phase = next(i for i, label in enumerate(order) if label.startswith("fast."))
    last_phase = max(i for i, label in enumerate(order) if label.startswith("glowup."))
    assert order.index("precheck") < first_phase
    assert last_phase < order.index("confirm-head")

    profile = _release_plan("release-profile", "nightly", "code").describe()
    assert "release-binaries.py --precheck" not in profile


def test_binary_precheck_requires_notes_only_for_a_new_identity(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    publish = FakeRunner(tmp_path)
    rebuild = FakeRunner(tmp_path, existing_version_tag=f"v{OLDER_VERSION}")

    RELEASE.precheck_release_binaries("stable", publish)
    RELEASE.precheck_release_binaries("nightly", rebuild)

    notes = (sys.executable, "scripts/extract-release-notes.py", "--check")
    assert notes in publish.calls
    assert notes not in rebuild.calls


def test_binary_precheck_rejects_stable_reuse_of_an_existing_version(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, existing_version_tag=f"v{OLDER_VERSION}")

    with pytest.raises(RuntimeError, match="bump the stable version"):
        RELEASE.precheck_release_binaries("stable", runner)


def test_binary_recipe_fetches_serialized_channel_source_before_full_local_gate() -> None:
    """The manifest is the bible, and it is read before the gate spends hours.

    Fetched fresh: a cached copy would let a release run against membership a
    concurrent release had already changed.
    """
    plan = _release_plan("release-binaries", "nightly")
    order = list(plan.labels)
    rendering = plan.describe()

    assert order.index("channel-source") < next(
        i for i, label in enumerate(order) if label.startswith("fast.")
    )
    assert "fetch-channel-source-manifest.py" in rendering
    assert "--channel nightly" in rendering
    assert "--require-profile-membership" in rendering
    assert "--bootstrap-missing-first-party" not in rendering
    assert "--repository" in rendering


def test_unexpected_write_aborts_before_commit_and_restores_owned_files(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    before = {path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS}
    runner = FakeRunner(tmp_path, unexpected=True)

    with pytest.raises(RuntimeError, match="write set is invalid"):
        RELEASE.release_binaries("stable", runner)

    assert not any(call[:2] == ("git", "commit") for call in runner.calls)
    assert {path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS} == before


@pytest.mark.parametrize("failure", ["commit", "tag"])
def test_git_preparation_failure_restores_owned_files_index_and_head(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    failure: str,
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    before = {path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS}
    runner = FakeRunner(
        tmp_path,
        fail_commit=failure == "commit",
        fail_tag=failure == "tag",
    )

    with pytest.raises(subprocess.CalledProcessError):
        RELEASE.release_binaries("nightly", runner)

    assert ("git", "reset", "--mixed", "a" * 40) in runner.calls
    assert runner.head == "a" * 40
    assert {path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS} == before
    assert not any(call[:2] == ("git", "push") for call in runner.calls)


def test_binary_release_accepts_a_cohort_file_that_did_not_need_rewriting(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A stamp that rewrites nothing is the normal case, not a failure.

    The version is a human decision committed ahead of the release, so by the
    time the release runs the cohort usually already carries it and stamping is
    pure propagation. Requiring every version file to change would fail every
    ordinary release. A *stale* lock is still rejected -- by its contents, in
    `test_binary_release_rejects_a_mixed_capsem_version_cohort`, which is the
    stronger check because it reads the version rather than the file's mtime.
    """
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, omit_cargo_lock_change=True)

    tag, _ = RELEASE.release_binaries("nightly", runner)

    assert tag == f"v{COHORT_VERSION}"
    assert any(call[:2] == ("git", "commit") for call in runner.calls)


def test_binary_release_requires_the_release_notes_to_be_written(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The notes are the one part of the write set that must change."""
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, omit_release_notes_change=True)

    with pytest.raises(RuntimeError, match=r"CHANGELOG.md"):
        RELEASE.release_binaries("nightly", runner)

    assert not any(call[:2] == ("git", "commit") for call in runner.calls)


def test_binary_release_rejects_a_mixed_capsem_version_cohort(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, mixed_version_cohort=True)

    with pytest.raises(RuntimeError, match="version cohort"):
        RELEASE.release_binaries("nightly", runner)

    assert not any(call[:2] == ("git", "commit") for call in runner.calls)


def test_invalid_channel_is_rejected_before_git_or_github(tmp_path: Path) -> None:
    runner = FakeRunner(tmp_path)

    with pytest.raises(ValueError, match="stable or nightly"):
        RELEASE.release_binaries("corp", runner)

    assert runner.calls == []


def test_nightly_release_rebuilds_when_main_has_no_unreleased_binary_change(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, current_release_tag=f"v{OLDER_VERSION}")

    tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert tag == f"v{OLDER_VERSION}"
    assert run_id == "42"
    assert runner.dispatched_ref == f"v{OLDER_VERSION}"
    assert runner.dispatched_publish == "false"
    assert ("just", "_stamp-version") not in runner.calls
    assert not any(call[:2] == ("git", "commit") for call in runner.calls)
    assert any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)


def test_untagged_nightly_main_rebuilds_an_existing_version_without_publishing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(
        tmp_path,
        existing_version_tag=f"v{OLDER_VERSION}",
    )

    tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert (tag, run_id) == (f"v{OLDER_VERSION}", "42")
    assert runner.dispatched_ref == "main"
    assert runner.dispatched_publish == "false"
    assert (sys.executable, "scripts/extract-release-notes.py", "--check") not in runner.calls
    assert ("just", "_stamp-version") not in runner.calls
    assert not any(call[:2] == ("git", "commit") for call in runner.calls)


def test_tagged_nightly_with_missing_dispatch_resumes_without_new_version(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    tag = f"v{OLDER_VERSION}"
    runner = FakeRunner(
        tmp_path,
        current_release_tag=tag,
        current_release_channel="nightly",
        run_rows=[],
    )

    resumed_tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert (resumed_tag, run_id) == (tag, "42")
    assert ("just", "_stamp-version") not in runner.calls
    assert any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)
    assert ("gh", "run", "watch", "42", "--exit-status") in runner.calls


def test_tagged_failed_nightly_stops_for_diagnosis_without_blind_rerun(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    tag = f"v{OLDER_VERSION}"
    runner = FakeRunner(
        tmp_path,
        current_release_tag=tag,
        current_release_channel="nightly",
        run_rows=[
            {
                "databaseId": 17,
                "displayTitle": f"Release nightly {tag}",
                "status": "completed",
                "conclusion": "failure",
            },
            {
                "databaseId": 99,
                "displayTitle": f"Release stable {tag}",
                "status": "completed",
                "conclusion": "success",
            },
        ],
    )

    with pytest.raises(
        RuntimeError,
        match=rf"nightly/v{re.escape(OLDER_VERSION)}.*run 17.*diagnose",
    ):
        RELEASE.release_binaries("nightly", runner)

    assert ("gh", "run", "rerun", "17") not in runner.calls
    assert ("gh", "run", "watch", "17", "--exit-status") not in runner.calls
    assert ("just", "_stamp-version") not in runner.calls


def test_tagged_successful_nightly_starts_a_fresh_rebuild(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    tag = f"v{OLDER_VERSION}"
    runner = FakeRunner(
        tmp_path,
        current_release_tag=tag,
        current_release_channel="nightly",
        run_rows=[
            {
                "databaseId": 23,
                "displayTitle": f"Release nightly {tag}",
                "status": "completed",
                "conclusion": "success",
            }
        ],
    )

    assert RELEASE.release_binaries("nightly", runner) == (tag, "42")
    assert any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)
    assert not any(call[:3] == ("gh", "run", "rerun") for call in runner.calls)
    assert ("gh", "run", "watch", "42", "--exit-status") in runner.calls
    assert runner.dispatched_publish == "false"


def test_unpushed_local_release_commit_is_pushed_and_dispatched_without_restamping(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    tag = f"v{OLDER_VERSION}"
    runner = FakeRunner(
        tmp_path,
        current_release_tag=tag,
        current_release_channel="stable",
        pending_local_release=True,
        run_rows=[],
    )

    resumed_tag, run_id = RELEASE.release_binaries("stable", runner)

    assert (resumed_tag, run_id) == (tag, "42")
    assert ("git", "push", "--atomic", "origin", "main", tag) in runner.calls
    assert ("just", "_stamp-version") not in runner.calls
    assert not any(call[:2] == ("git", "commit") for call in runner.calls)


def test_divergent_local_release_commit_is_not_force_pushed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(
        tmp_path,
        current_release_tag=f"v{OLDER_VERSION}",
        current_release_channel="stable",
        pending_local_release=True,
        divergent_pending_release=True,
    )

    with pytest.raises(ValueError, match="not one resumable stable release commit"):
        RELEASE.release_binaries("stable", runner)

    assert not any(call[:2] == ("git", "push") for call in runner.calls)


def test_stable_binary_release_remains_explicit_even_at_a_version_tag(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(
        tmp_path,
        current_release_tag=f"v{OLDER_VERSION}",
        current_release_channel="nightly",
    )

    tag, run_id = RELEASE.release_binaries("stable", runner)

    assert tag == f"v{COHORT_VERSION}"
    assert run_id == "42"
    assert ("just", "_stamp-version") in runner.calls
    assert any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)
    assert runner.dispatched_ref == tag
    assert runner.dispatched_channel == "stable"
    assert runner.dispatched_publish == "true"


def test_daily_nightly_schedule_uses_only_public_orthogonal_commands() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-nightly.yaml").read_text(
        encoding="utf-8"
    )

    assert workflow.count("cron:") == 1
    assert '- cron: "23 7 * * *"' in workflow
    assert "push:" not in workflow
    assert workflow.count("just release-binaries nightly") == 1
    assert workflow.count("just release-profile nightly ${{ matrix.profile }}") == 1
    assert "profile: [code, co-work]" in workflow
    assert "max-parallel: 1" in workflow
    assert "fail-fast: false" in workflow
    assert "group: capsem-nightly-release-scheduler" in workflow
    assert "cancel-in-progress: false" in workflow
    assert "ref: main" in workflow
    assert "fetch-depth: 0" in workflow
    binary_job = workflow.split("  release-binaries:", maxsplit=1)[1]
    assert "needs: release-profiles" in binary_job
    assert "if: ${{ always() }}" in binary_job
    for forbidden in (
        "fetch-channel-source-manifest.py",
        "capsem-admin",
        "_build-kernel",
        "_build-rootfs",
        "build-pkg.sh",
        "build-complete-release-channel.py",
        "release-channel.yaml",
        "release.yaml",
        "release-assets.yaml",
    ):
        assert forbidden not in workflow


def test_release_notes_promote_unreleased_once() -> None:
    changelog = """# Changelog

## [Unreleased]

### Fixed

- Kept the release lanes orthogonal.

## [1.4.0] - 2026-07-01

- Previous release.
"""

    updated, body = NOTES.promote_release(changelog, "1.5.0", "2026-07-24")

    assert updated.count("## [Unreleased]") == 1
    assert "## [1.5.0] - 2026-07-24" in updated
    assert "## [1.4.0] - 2026-07-01" in updated
    assert body == "### Fixed\n\n- Kept the release lanes orthogonal."

    with pytest.raises(ValueError, match="already contains release"):
        NOTES.promote_release(updated, "1.5.0", "2026-07-24")


def test_release_notes_validate_unreleased_without_mutation() -> None:
    changelog = """# Changelog

## [Unreleased]

### Fixed

- Fail release prerequisites before expensive work.

## [1.4.0] - 2026-07-01
"""

    assert NOTES.validate_unreleased(changelog) == (
        "### Fixed\n\n- Fail release prerequisites before expensive work."
    )


@pytest.mark.parametrize(
    ("changelog", "message"),
    [
        ("# Changelog\n", r"no \[Unreleased\]"),
        (
            "# Changelog\n\n## [Unreleased]\n\n## [1.4.0] - 2026-07-01\n",
            r"\[Unreleased\] section is empty",
        ),
    ],
)
def test_release_notes_fail_closed(changelog: str, message: str) -> None:
    with pytest.raises(ValueError, match=message):
        NOTES.promote_release(changelog, "1.5.0", "2026-07-24")
