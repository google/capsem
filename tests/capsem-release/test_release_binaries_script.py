from __future__ import annotations

import importlib.util
import json
import re
from pathlib import Path
import subprocess
import sys
from typing import Sequence

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "release-binaries.py"
SPEC = importlib.util.spec_from_file_location("release_binaries", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
RELEASE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RELEASE
SPEC.loader.exec_module(RELEASE)
NOTES_SCRIPT = PROJECT_ROOT / "scripts" / "extract-release-notes.py"
NOTES_SPEC = importlib.util.spec_from_file_location(
    "extract_release_notes", NOTES_SCRIPT
)
assert NOTES_SPEC is not None and NOTES_SPEC.loader is not None
NOTES = importlib.util.module_from_spec(NOTES_SPEC)
NOTES_SPEC.loader.exec_module(NOTES)

# Fixture versions follow the release line the script enforces rather than
# restating it. Hardcoded 1.6.x fixtures failed this whole suite the moment the
# line moved to 0.6, reporting an inconsistent cohort instead of a stale test.
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
        mixed_version_cohort: bool = False,
        current_release_channel: str | None = None,
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
        self.mixed_version_cohort = mixed_version_cohort
        self.current_release_channel = current_release_channel
        self.run_rows = run_rows
        self.pending_local_release = pending_local_release
        self.divergent_pending_release = divergent_pending_release
        self.fail_commit = fail_commit
        self.fail_tag = fail_tag
        self.calls: list[tuple[str, ...]] = []
        self.stamped = False
        self.dispatched_tag = ""
        self.dispatched_channel = ""
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
            if self.unexpected:
                paths.append(Path("config/profiles/code/profile.toml"))
            return RELEASE.CommandResult(
                "\n".join(f" M {path}" for path in paths)
            )
        if command == ("git", "branch", "--show-current"):
            return RELEASE.CommandResult("main\n")
        if command in (
            ("git", "rev-parse", "HEAD"),
        ):
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
                "[[package]]\n"
                'name = "capsem"\n'
                f'version = "{COHORT_VERSION}"\n',
                encoding="utf-8",
            )
            return RELEASE.CommandResult("")
        if len(command) >= 2 and command[1] == "scripts/extract-release-notes.py":
            return RELEASE.CommandResult("")
        if command[:3] == ("git", "tag", "--list"):
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
            fields = [
                command[index + 1]
                for index, value in enumerate(command)
                if value == "-f"
            ]
            self.dispatched_channel = next(
                value.removeprefix("channel=")
                for value in fields
                if value.startswith("channel=")
            )
            return RELEASE.CommandResult("")
        if command[:4] == ("gh", "run", "list", "--workflow"):
            rows = self.run_rows
            if rows is None:
                rows = []
            if not rows and self.dispatched_tag:
                tag = self.dispatched_tag or self.current_release_tag or f"v{COHORT_VERSION}"
                channel = self.dispatched_channel or self.current_release_channel or "nightly"
                rows = [
                    {
                        "databaseId": 42,
                        "displayTitle": f"Release {channel} {tag}",
                        "status": "in_progress",
                        "conclusion": "",
                    }
                ]
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
    ) in runner.calls
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

    check = runner.calls.index(
        (sys.executable, "scripts/extract-release-notes.py", "--check")
    )
    stamp = runner.calls.index(("just", "_stamp-version"))
    assert check < stamp


def test_binary_recipe_checks_notes_before_complete_local_gate_and_push() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    recipe = justfile.split("\nrelease-binaries channel:", 1)[1].split(
        "\nrelease-profile channel profile:", 1
    )[0]

    check = recipe.index("python3 scripts/extract-release-notes.py --check")
    test = recipe.index("just test")
    # The publishing invocation, not the --precheck precondition that now
    # runs before the gate; both share a script name.
    push = recipe.index("publish-tested-main.py --expected-head")
    assert check < test < push
    assert "extract-release-notes.py" not in justfile.split(
        "\nrelease-profile channel profile:", 1
    )[1].split("\n# Compile all host binaries", 1)[0]


def test_binary_recipe_fetches_serialized_channel_source_before_full_local_gate() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    recipe = justfile.split("\nrelease-binaries channel:", 1)[1].split(
        "\nrelease-profile channel profile:", 1
    )[0]

    fetch = recipe.index("scripts/fetch-channel-source-manifest.py")
    full_gate = recipe.index("just test")
    assert fetch < full_gate
    assert '--channel "{{channel}}"' in recipe
    assert '--repository "$RELEASE_REPOSITORY"' in recipe
    assert 'RELEASE_REPOSITORY="${GITHUB_REPOSITORY:-' in recipe
    assert "--bootstrap-missing-first-party" not in recipe
    assert "--require-profile-membership" in recipe
    assert "GITHUB_TOKEN" in recipe


def test_unexpected_write_aborts_before_commit_and_restores_owned_files(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    before = {
        path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS
    }
    runner = FakeRunner(tmp_path, unexpected=True)

    with pytest.raises(RuntimeError, match="write set is invalid"):
        RELEASE.release_binaries("stable", runner)

    assert not any(call[:2] == ("git", "commit") for call in runner.calls)
    assert {
        path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS
    } == before


@pytest.mark.parametrize("failure", ["commit", "tag"])
def test_git_preparation_failure_restores_owned_files_index_and_head(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    failure: str,
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    before = {
        path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS
    }
    runner = FakeRunner(
        tmp_path,
        fail_commit=failure == "commit",
        fail_tag=failure == "tag",
    )

    with pytest.raises(subprocess.CalledProcessError):
        RELEASE.release_binaries("nightly", runner)

    assert ("git", "reset", "--mixed", "a" * 40) in runner.calls
    assert runner.head == "a" * 40
    assert {
        path: (tmp_path / path).read_bytes() for path in RELEASE.MUTATED_PATHS
    } == before
    assert not any(call[:2] == ("git", "push") for call in runner.calls)


def test_binary_release_requires_cargo_lock_to_join_the_version_cut(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, omit_cargo_lock_change=True)

    with pytest.raises(RuntimeError, match="Cargo.lock"):
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


def test_nightly_release_skips_when_main_has_no_unreleased_binary_change(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, current_release_tag=f"v{OLDER_VERSION}")

    tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert tag is None
    assert run_id is None
    assert ("just", "_stamp-version") not in runner.calls
    assert not any(call[:2] == ("git", "commit") for call in runner.calls)
    assert not any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)


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


def test_tagged_successful_nightly_is_idempotent(
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

    assert RELEASE.release_binaries("nightly", runner) == (tag, "23")
    assert not any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)
    assert not any(call[:3] == ("gh", "run", "rerun") for call in runner.calls)
    assert not any(call[:3] == ("gh", "run", "watch") for call in runner.calls)


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
        "channel=stable",
    ) in runner.calls


def test_daily_nightly_schedule_uses_only_the_public_binary_command() -> None:
    workflow = (
        PROJECT_ROOT / ".github" / "workflows" / "release-nightly.yaml"
    ).read_text(encoding="utf-8")

    assert workflow.count("cron:") == 1
    assert '- cron: "23 7 * * *"' in workflow
    assert "push:" not in workflow
    assert workflow.count("just release-binaries nightly") == 1
    assert "group: capsem-nightly-release-scheduler" in workflow
    assert "cancel-in-progress: false" in workflow
    assert "ref: main" in workflow
    assert "fetch-depth: 0" in workflow
    assert "release.yaml" not in workflow
    assert "release-assets.yaml" not in workflow
    for forbidden in (
        "fetch-channel-source-manifest.py",
        "capsem-admin",
        "_build-kernel",
        "_build-rootfs",
        "build-pkg.sh",
        "build-complete-release-channel.py",
        "release-channel.yaml",
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
