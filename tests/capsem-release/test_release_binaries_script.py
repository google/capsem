from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

import pytest
from capsem_builder.release.tools import release_version_tag as TAGS

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
SOURCE = "0123456789abcdef0123456789abcdef01234567"
VERSION = f"{RELEASE.RELEASE_LINE}.2"
TAG = f"v{VERSION}"


def _prepared_tree(root: Path) -> None:
    (root / "crates/capsem-app").mkdir(parents=True)
    (root / "build_system").mkdir()
    (root / "Cargo.toml").write_text(
        f'[workspace.package]\nversion = "{VERSION}"\n', encoding="utf-8"
    )
    packages = "".join(
        f'[[package]]\nname = "{name}"\nversion = "{VERSION}"\n\n'
        for name in ("capsem", "capsem-core")
    )
    (root / "Cargo.lock").write_text(packages, encoding="utf-8")
    (root / "crates/capsem-app/tauri.conf.json").write_text(
        json.dumps({"version": VERSION}), encoding="utf-8"
    )
    (root / "build_system/pyproject.toml").write_text(
        f'[project]\nversion = "{VERSION}"\n', encoding="utf-8"
    )
    (root / "build_system/uv.lock").write_text(
        f'[[package]]\nname = "capsem-builder"\nversion = "{VERSION}"\n', encoding="utf-8"
    )
    body = "### Fixed\n\n- Qualify one committed immutable source."
    (root / "CHANGELOG.md").write_text(
        f"# Changelog\n\n## [Unreleased]\n\n{body}\n\n"
        f"## [{VERSION}] - 2020-01-01\n\n- Historical development snapshot.\n",
        encoding="utf-8",
    )
    (root / "LATEST_RELEASE.md").write_text(f"version: {VERSION}\n---\n{body}\n", encoding="utf-8")


class FakeRunner:
    def __init__(
        self, *, version_target: str | None = None, release_published: bool = False
    ) -> None:
        self.version_target = version_target
        self.release_published = release_published
        self.calls: list[tuple[str, ...]] = []
        self.dispatched = False
        self.dispatch_id = ""

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
            return RELEASE.CommandResult("")
        if command == ("git", "branch", "--show-current"):
            return RELEASE.CommandResult("")
        if command == ("git", "rev-parse", "HEAD"):
            return RELEASE.CommandResult(SOURCE)
        if command[:3] == ("git", "tag", "--list"):
            return RELEASE.CommandResult(TAG if self.version_target is not None else "")
        if command[:4] == ("git", "ls-remote", "--tags", "origin"):
            if self.version_target is None:
                return RELEASE.CommandResult("")
            return RELEASE.CommandResult(
                f"{self.version_target}\trefs/tags/{TAG}\n"
                f"{self.version_target}\trefs/tags/{TAG}^{{}}\n"
            )
        if command[:3] == ("gh", "api", "graphql"):
            release = {"tagName": TAG} if self.release_published else None
            return RELEASE.CommandResult(
                json.dumps({"data": {"repository": {"release": release}}})
            )
        if "tag" in command and "-a" in command:
            return RELEASE.CommandResult("")
        if command[:3] == ("git", "push", "origin"):
            self.version_target = SOURCE
            return RELEASE.CommandResult("")
        if command[:3] == ("gh", "workflow", "run"):
            fields = [command[index + 1] for index, part in enumerate(command) if part == "-f"]
            self.dispatch_id = next(
                field.removeprefix("dispatch_id=")
                for field in fields
                if field.startswith("dispatch_id=")
            )
            self.dispatched = True
            return RELEASE.CommandResult("")
        if command[:4] == ("gh", "run", "list", "--workflow"):
            rows = []
            if self.dispatched:
                rows.append(
                    {
                        "databaseId": 42,
                        "displayTitle": f"Release nightly {TAG} {self.dispatch_id}",
                        "headSha": SOURCE,
                        "headBranch": f"capsem-source-{SOURCE}",
                        "status": "in_progress",
                        "conclusion": "",
                    }
                )
            return RELEASE.CommandResult(json.dumps(rows))
        if command[:4] == ("gh", "run", "view", "42"):
            return RELEASE.CommandResult(
                json.dumps(
                    {
                        "databaseId": 42,
                        "displayTitle": f"Release nightly {TAG} {self.dispatch_id}",
                        "headSha": SOURCE,
                        "headBranch": f"capsem-source-{SOURCE}",
                        "status": "completed",
                        "conclusion": "success",
                    }
                )
            )
        return RELEASE.CommandResult("")


def test_release_script_never_edits_or_pushes_tracked_source() -> None:
    source = SCRIPT.read_text(encoding="utf-8")

    for forbidden in (
        "OwnedMutation",
        "MUTATED_PATHS",
        '"_stamp-version"',
        '"commit"',
        '"reset"',
        '"push", "--atomic", "origin", "main"',
        'ref="main"',
    ):
        assert forbidden not in source


def test_prepared_commit_creates_only_refs_then_dispatches_exact_source(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _prepared_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner()

    tag, run_id = RELEASE.release_binaries("nightly", SOURCE, runner)

    assert (tag, run_id) == (TAG, "42")
    tag = next(call for call in runner.calls if "tag" in call and "-a" in call)
    assert f"user.name={RELEASE.TAGGER_NAME}" in tag
    assert f"user.email={RELEASE.TAGGER_EMAIL}" in tag
    assert tag[-6:] == (
        "tag",
        "-a",
        TAG,
        SOURCE,
        "-m",
        f"Capsem {VERSION} channel=nightly",
    )
    assert ("git", "push", "origin", f"refs/tags/{TAG}:refs/tags/{TAG}") in runner.calls
    dispatch = next(call for call in runner.calls if call[:3] == ("gh", "workflow", "run"))
    assert dispatch[dispatch.index("--ref") + 1] == f"capsem-source-{SOURCE}"
    assert f"source_commit={SOURCE}" in dispatch
    assert runner.calls[-1][:4] == ("gh", "run", "view", "42")


def test_binary_precheck_does_not_require_release_notes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _prepared_tree(tmp_path)
    (tmp_path / "CHANGELOG.md").unlink()
    (tmp_path / "LATEST_RELEASE.md").unlink()
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)

    RELEASE.precheck_release_binaries("stable", SOURCE, FakeRunner())


def test_existing_version_from_another_source_is_nightly_proof_only(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _prepared_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(version_target="f" * 40)

    RELEASE.release_binaries("nightly", SOURCE, runner)

    dispatch = next(call for call in runner.calls if call[:3] == ("gh", "workflow", "run"))
    assert "publish=false" in dispatch
    assert not any("tag" in call and "-a" in call for call in runner.calls)


def test_stable_refuses_a_version_tag_from_another_source(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _prepared_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(version_target="f" * 40)

    with pytest.raises(RuntimeError, match="points at"):
        RELEASE.release_binaries("stable", SOURCE, runner)
    assert not runner.dispatched


def test_run_correlation_rejects_a_title_match_from_another_source() -> None:
    class Wrong(FakeRunner):
        def run(self, argv, **kwargs):
            result = super().run(argv, **kwargs)
            if tuple(argv)[:4] == ("gh", "run", "list", "--workflow") and self.dispatched:
                row = json.loads(result.stdout)[0]
                row["headSha"] = "f" * 40
                return RELEASE.CommandResult(json.dumps([row]))
            return result

    runner = Wrong(version_target=SOURCE)
    with pytest.raises(RuntimeError, match="malformed matching release run"):
        RELEASE._resume_release(
            runner,
            ref=f"capsem-source-{SOURCE}",
            source_commit=SOURCE,
            tag=TAG,
            channel="nightly",
            publish=True,
        )


def test_successful_resume_rechecks_the_exact_run_identity() -> None:
    class Completed(FakeRunner):
        def run(self, argv, **kwargs):
            command = tuple(argv)
            if command[:4] == ("gh", "run", "list", "--workflow"):
                self.calls.append(command)
                return RELEASE.CommandResult(
                    json.dumps(
                        [
                            {
                                "databaseId": 42,
                                "displayTitle": f"Release nightly {TAG}",
                                "headSha": SOURCE,
                                "headBranch": f"capsem-source-{SOURCE}",
                                "status": "completed",
                                "conclusion": "success",
                            }
                        ]
                    )
                )
            if command[:4] == ("gh", "run", "view", "42"):
                self.calls.append(command)
                return RELEASE.CommandResult(
                    json.dumps(
                        {
                            "databaseId": 42,
                            "displayTitle": f"Release nightly {TAG}",
                            "headSha": SOURCE,
                            "headBranch": f"capsem-source-{SOURCE}",
                            "status": "completed",
                            "conclusion": "success",
                        }
                    )
                )
            return super().run(argv, **kwargs)

    runner = Completed(version_target=SOURCE)
    run_id = RELEASE._resume_release(
        runner,
        ref=f"capsem-source-{SOURCE}",
        source_commit=SOURCE,
        tag=TAG,
        channel="nightly",
        publish=True,
    )

    assert run_id == "42"
    assert runner.calls[-1][:4] == ("gh", "run", "view", "42")
    assert not any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)


def test_transient_watch_failure_reattaches_to_the_same_run() -> None:
    class TransientWatch(FakeRunner):
        def __init__(self) -> None:
            super().__init__(version_target=SOURCE)
            self.watch_count = 0
            self.view_count = 0

        def run(self, argv, **kwargs):
            command = tuple(argv)
            if command[:4] == ("gh", "run", "list", "--workflow"):
                self.calls.append(command)
                return RELEASE.CommandResult(
                    json.dumps(
                        [
                            {
                                "databaseId": 42,
                                "displayTitle": f"Release nightly {TAG}",
                                "headSha": SOURCE,
                                "headBranch": f"capsem-source-{SOURCE}",
                                "status": "in_progress",
                                "conclusion": "",
                            }
                        ]
                    )
                )
            if command == ("gh", "run", "watch", "42", "--exit-status"):
                self.calls.append(command)
                self.watch_count += 1
                if self.watch_count == 1:
                    raise subprocess.CalledProcessError(1, command)
                return RELEASE.CommandResult("")
            if command[:4] == ("gh", "run", "view", "42"):
                self.calls.append(command)
                self.view_count += 1
                completed = self.view_count > 1
                return RELEASE.CommandResult(
                    json.dumps(
                        {
                            "databaseId": 42,
                            "displayTitle": f"Release nightly {TAG}",
                            "headSha": SOURCE,
                            "headBranch": f"capsem-source-{SOURCE}",
                            "status": "completed" if completed else "in_progress",
                            "conclusion": "success" if completed else "",
                        }
                    )
                )
            return super().run(argv, **kwargs)

    runner = TransientWatch()

    run_id = RELEASE._resume_release(
        runner,
        ref=f"capsem-source-{SOURCE}",
        source_commit=SOURCE,
        tag=TAG,
        channel="nightly",
        publish=True,
    )

    assert run_id == "42"
    assert runner.watch_count == 2
    assert not any(call[:3] == ("gh", "workflow", "run") for call in runner.calls)


def test_transient_watch_failure_still_fails_closed_for_a_red_run() -> None:
    class FailedWatch(FakeRunner):
        def run(self, argv, **kwargs):
            command = tuple(argv)
            if command == ("gh", "run", "watch", "42", "--exit-status"):
                self.calls.append(command)
                raise subprocess.CalledProcessError(1, command)
            if command[:4] == ("gh", "run", "view", "42"):
                self.calls.append(command)
                return RELEASE.CommandResult(
                    json.dumps(
                        {
                            "databaseId": 42,
                            "displayTitle": f"Release nightly {TAG}",
                            "headSha": SOURCE,
                            "headBranch": f"capsem-source-{SOURCE}",
                            "status": "completed",
                            "conclusion": "failure",
                        }
                    )
                )
            return super().run(argv, **kwargs)

    runner = FailedWatch(version_target=SOURCE)
    run = RELEASE.ReleaseRun(
        "42",
        f"Release nightly {TAG}",
        SOURCE,
        f"capsem-source-{SOURCE}",
        "in_progress",
        "",
    )

    with pytest.raises(RuntimeError, match="completed with failure"):
        RELEASE._complete_run(
            runner,
            run,
            TAG,
            "nightly",
            SOURCE,
            f"capsem-source-{SOURCE}",
        )


def test_remote_version_tag_rejects_duplicate_rows() -> None:
    class Duplicate(FakeRunner):
        def run(self, argv, **kwargs):
            command = tuple(argv)
            if command[:4] == ("git", "ls-remote", "--tags", "origin"):
                return RELEASE.CommandResult(
                    f"{SOURCE}\trefs/tags/{TAG}\n{SOURCE}\trefs/tags/{TAG}\n"
                )
            return super().run(argv, **kwargs)

    with pytest.raises(RuntimeError, match="duplicate rows"):
        RELEASE.remote_version_target(Duplicate(), TAG)


def test_daily_nightly_schedule_freezes_one_scheduler_commit() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-nightly.yaml").read_text(encoding="utf-8")

    assert workflow.count("cron:") == 1
    # One job, so one checkout: the qualification journal `just test-clean` writes is
    # machine-local, and every release command in this workflow reads it back.
    assert workflow.count("ref: ${{ github.sha }}") == 1
    # The scheduler qualifies nothing: spec 13.2 rebuilds current `main`, and
    # the dispatched lanes prove themselves before publishing.
    assert "just test-clean" not in workflow
    assert workflow.count("scripts/nightly_release_scheduler.py") == 1
    assert workflow.count('--source-commit "${{ github.sha }}"') == 1
    assert "just release-" not in workflow
    for profile in ("code", "co-work"):
        assert workflow.count(f"--profile {profile}") == 1
    assert "ref: main" not in workflow


def test_release_notes_prepare_without_claiming_the_tag_exists() -> None:
    changelog = """# Changelog

## [Unreleased]

### Fixed

- Kept the release lanes orthogonal.

## [1.5.0] - 2026-07-01

- Historical development snapshot with the same version.
"""

    rendered = NOTES.render_release_notes(changelog, "1.5.0")

    assert rendered == ("version: 1.5.0\n---\n### Fixed\n\n- Kept the release lanes orthogonal.\n")
    assert "## [Unreleased]" in changelog
    assert "Historical development snapshot" in changelog


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
        NOTES.render_release_notes(changelog, "1.5.0")


def test_a_failed_release_does_not_leave_the_version_burned(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The tag has to exist before the build, and must not outlive its failure.

    CI attaches the release to the tag, so the tag is pushed first. Nothing
    undid it when the build failed, and the same guard that protects an
    immutable identity then refuses to reuse it: one failed macOS step burned
    v0.6.0, leaving a tag that pointed at a commit and promised artifacts that
    were never built.
    """
    _prepared_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner()

    def explode(*args, **kwargs):
        raise RuntimeError("the macOS job failed")

    monkeypatch.setattr(RELEASE, "_resume_release", explode)

    with pytest.raises(RuntimeError, match="macOS job failed"):
        RELEASE.release_binaries("stable", SOURCE, runner)

    assert ("git", "push", "origin", f":refs/tags/{TAG}") in runner.calls, (
        "a tag this invocation created must not survive the failure that "
        "followed it, or the version can never be released again"
    )


def test_a_version_that_already_existed_survives_a_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Only clean up what this run created. A tag that was already published
    belongs to whatever published it, and deleting it would be the far worse
    bug."""
    _prepared_tree(tmp_path)
    monkeypatch.setattr(RELEASE, "ROOT", tmp_path)
    runner = FakeRunner(version_target=SOURCE)

    def explode(*args, **kwargs):
        raise RuntimeError("the macOS job failed")

    monkeypatch.setattr(RELEASE, "_resume_release", explode)

    with pytest.raises(RuntimeError, match="macOS job failed"):
        RELEASE.release_binaries("stable", SOURCE, runner)

    assert ("git", "push", "origin", f":refs/tags/{TAG}") not in runner.calls


def test_hosted_cleanup_deletes_only_the_exact_unpublished_claim() -> None:
    runner = FakeRunner(version_target=SOURCE)

    deleted = TAGS.discard_exact_unpublished_claim(
        runner,
        tag=TAG,
        source_commit=SOURCE,
        repository="google/capsem",
    )

    assert deleted
    assert ("git", "push", "origin", f":refs/tags/{TAG}") in runner.calls


def test_hosted_cleanup_refuses_an_unrelated_version_tag() -> None:
    unrelated = "f" * 40
    runner = FakeRunner(version_target=unrelated)

    with pytest.raises(RuntimeError, match="refusing to discard"):
        TAGS.discard_exact_unpublished_claim(
            runner,
            tag=TAG,
            source_commit=SOURCE,
            repository="google/capsem",
        )

    assert ("git", "push", "origin", f":refs/tags/{TAG}") not in runner.calls


def test_hosted_cleanup_keeps_a_published_exact_version_tag() -> None:
    runner = FakeRunner(version_target=SOURCE, release_published=True)

    deleted = TAGS.discard_exact_unpublished_claim(
        runner,
        tag=TAG,
        source_commit=SOURCE,
        repository="google/capsem",
    )

    assert not deleted
    assert ("git", "push", "origin", f":refs/tags/{TAG}") not in runner.calls
