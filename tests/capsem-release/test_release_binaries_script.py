from __future__ import annotations

import importlib.util
from pathlib import Path
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


class FakeRunner:
    def __init__(
        self,
        root: Path,
        *,
        unexpected: bool = False,
        current_release_tag: str = "",
        omit_cargo_lock_change: bool = False,
        mixed_version_cohort: bool = False,
    ) -> None:
        self.root = root
        self.unexpected = unexpected
        self.current_release_tag = current_release_tag
        self.omit_cargo_lock_change = omit_cargo_lock_change
        self.mixed_version_cohort = mixed_version_cohort
        self.calls: list[tuple[str, ...]] = []
        self.stamped = False

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
            ("git", "rev-parse", "origin/main"),
        ):
            return RELEASE.CommandResult("a" * 40 + "\n")
        if command == ("git", "tag", "--points-at", "HEAD", "--list", "v*"):
            return RELEASE.CommandResult(f"{self.current_release_tag}\n")
        if command == ("just", "_stamp-version"):
            self.stamped = True
            for path in RELEASE.MUTATED_PATHS:
                target = self.root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(f"changed {path}\n", encoding="utf-8")
            (self.root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "1.6.2000000000"\n',
                encoding="utf-8",
            )
            (self.root / "Cargo.lock").write_text(
                "[[package]]\n"
                'name = "capsem"\n'
                f'version = "{"1.5.1000000000" if self.mixed_version_cohort else "1.6.2000000000"}"\n'
                "\n[[package]]\n"
                'name = "capsem-core"\n'
                'version = "1.6.2000000000"\n',
                encoding="utf-8",
            )
            (self.root / "crates/capsem-app/tauri.conf.json").write_text(
                '{"version": "1.6.2000000000"}\n',
                encoding="utf-8",
            )
            (self.root / "pyproject.toml").write_text(
                '[project]\nversion = "1.6.2000000000"\n',
                encoding="utf-8",
            )
            (self.root / "uv.lock").write_text(
                "[[package]]\n"
                'name = "capsem"\n'
                'version = "1.6.2000000000"\n',
                encoding="utf-8",
            )
            return RELEASE.CommandResult("")
        if len(command) >= 2 and command[1] == "scripts/extract-release-notes.py":
            return RELEASE.CommandResult("")
        if command[:3] == ("git", "tag", "--list"):
            return RELEASE.CommandResult("")
        if command[:4] == ("gh", "run", "list", "--workflow"):
            return RELEASE.CommandResult("42\n")
        return RELEASE.CommandResult("")


def _release_tree(tmp_path: Path) -> None:
    for relative in RELEASE.MUTATED_PATHS:
        path = tmp_path / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"original {relative}\n", encoding="utf-8")
    (tmp_path / "Cargo.toml").write_text(
        '[workspace.package]\nversion = "1.6.1000000000"\n',
        encoding="utf-8",
    )


def test_binary_release_owns_one_scripted_build_and_dispatch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path)

    tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert tag == "v1.6.2000000000"
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
    runner = FakeRunner(tmp_path, current_release_tag="v1.6.1000000000")

    tag, run_id = RELEASE.release_binaries("nightly", runner)

    assert tag is None
    assert run_id is None
    assert ("just", "_stamp-version") not in runner.calls
    assert not any(call[:2] == ("git", "commit") for call in runner.calls)
    assert not any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)


def test_stable_binary_release_remains_explicit_even_at_a_version_tag(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _release_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(tmp_path, current_release_tag="v1.6.1000000000")

    tag, run_id = RELEASE.release_binaries("stable", runner)

    assert tag == "v1.6.2000000000"
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
