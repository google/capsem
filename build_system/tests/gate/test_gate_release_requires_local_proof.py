"""A release dispatches only a commit a passing local `just test` proved.

Four consecutive stable release attempts each failed in the hosted lane after
about two and a half hours, on a defect the local glow-up and functional lanes
would have caught in minutes. So both release commands consume the
qualification journal `just test <commit>` writes, and refuse before the
private prefix, the machine lock, tagging, pushing, or dispatching when no
passing journal covers the released commit's source -- the same commit, or
one with the identical Git tree (a pull request's merge commit).
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import qualificationevidence, qualificationflow, sourcecommit
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.execution import ResumePolicy, step
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.qualification import LocalQualification
from capsem_builder.gate.qualificationevidence import QualificationPolicy
from capsem_builder.gate.release import ReleaseBinariesCommand, ReleaseProfileCommand
from capsem_builder.gate.runlog import RunLog
from capsem_builder.gate.runlogschema import (
    FAILED,
    OK,
    QualificationComplete,
    RunEnd,
    StepEnd,
)
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
COMMIT = SourceCommit("5" * 40)

#: (command class, positional arguments before the commit).
RELEASES = {
    "release-binaries": (ReleaseBinariesCommand, {"channel": "stable"}),
    "release-profile": (ReleaseProfileCommand, {"channel": "stable", "profile": "code"}),
}


@pytest.fixture
def config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    subprocess.run(["git", "init", "--quiet", "-b", "main"], cwd=tmp_path, check=True)
    (tmp_path / "config").mkdir()
    shutil.copy(PROJECT_ROOT / "config/gate.toml", tmp_path / "config/gate.toml")
    loaded = gate_config.load(tmp_path)
    # This fixture owns a synthetic journal authority; an outer gate's prefix
    # markers would point evidence lookup at the real checkout instead.
    monkeypatch.delenv(loaded.environment.source_checkout, raising=False)
    monkeypatch.delenv(loaded.environment.source_commit, raising=False)
    monkeypatch.delenv(loaded.locks.gate.run_marker, raising=False)
    monkeypatch.setattr(qualificationflow, "require_local_main", lambda *_args: None)
    return loaded


def _candidate_plan() -> Plan:
    plan = Plan("candidate")
    recorded = plan.add(step("source.record", resume=ResumePolicy.ALWAYS_RUN))
    built = plan.add(step("build"), after=(recorded,))
    plan.add(step("source.verify"), after=(built,))
    return plan


def _record(
    config,
    monkeypatch: pytest.MonkeyPatch,
    *,
    commit: SourceCommit = COMMIT,
    status: str = OK,
    argv: tuple[str, ...] = (),
) -> None:
    """Write the exact journal a `just test <commit>` run leaves behind."""
    plan = _candidate_plan()
    monkeypatch.setattr("capsem_builder.gate.runlog.head_revision", lambda _root: str(commit))
    with RunLog.open(config, "candidate", argv=argv, source_commit=str(commit)) as log:
        log.qualification_attempt(commit)
        log.shape(plan.labels, plan.edges)
        for label in plan.labels:
            failed = status != OK and label == "build"
            log.emit(StepEnd(step=label, status=FAILED if failed else OK, duration_ms=1.0))
        if status == OK:
            log.emit(
                QualificationComplete(
                    source_commit=str(commit),
                    source_digest="b" * 64,
                    plan_digest=qualificationevidence.plan_digest(plan.labels, plan.edges),
                )
            )
        else:
            log.emit(RunEnd(status=FAILED, duration_ms=3.0, failures={"build": "boom"}))


def _release(
    config,
    name: str,
    *,
    channel: str | None = None,
    force: str = "false",
    commit: SourceCommit = COMMIT,
):
    kind, arguments = RELEASES[name]
    arguments = {**arguments, **({"channel": channel} if channel else {})}
    return kind(
        RecordingRunner(config.root),
        argparse.Namespace(
            **arguments,
            source_commit=commit,
            force=force,
            dry_run=False,
            graph=False,
            timing=False,
            prefix=None,
            resume_from=None,
            sandbox=None,
        ),
        qualification=LocalQualification(bin_dir=config.modules.default_bin_dir),
    )


def _dispatcher_plan() -> Plan:
    plan = Plan("release")
    plan.add(step("release"))
    return plan


def _execute(command, monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """Run to the first expensive boundary: the private-prefix re-exec."""
    reached: list[str] = []

    def private_copy(*_args, **_kwargs) -> int:
        reached.append("prefix")
        return 0

    monkeypatch.setattr(command, "_describe", _dispatcher_plan)
    monkeypatch.setattr("capsem_builder.gate.command.prefix.active", lambda *_args: False)
    monkeypatch.setattr("capsem_builder.gate.command.prefix.run_from_private_copy", private_copy)
    monkeypatch.setattr(command, "reexec", lambda: pytest.fail("sandbox re-exec reached"))
    monkeypatch.setattr(command, "resources", lambda *_args: pytest.fail("resources reached"))
    with pytest.raises(SystemExit) as exited:
        command.execute()
    assert exited.value.code == 0
    return reached


def _refused(command, monkeypatch: pytest.MonkeyPatch, commit: SourceCommit) -> None:
    monkeypatch.setattr(command, "_describe", _dispatcher_plan)
    monkeypatch.setattr(
        "capsem_builder.gate.command.prefix.run_from_private_copy",
        lambda *_a, **_k: pytest.fail("the release reached its private prefix without proof"),
    )
    with pytest.raises(GateError) as refused:
        command.execute()
    assert f"`just test {commit}`" in str(refused.value)
    assert command._runner.commands == []


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_release_refuses_without_a_passing_local_test(config, monkeypatch, name) -> None:
    _refused(_release(config, name), monkeypatch, COMMIT)


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_release_proceeds_with_the_exact_passing_journal(config, monkeypatch, name) -> None:
    _record(config, monkeypatch)

    assert _execute(_release(config, name), monkeypatch) == ["prefix"]


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_an_approved_forced_local_test_that_passes_is_proof(config, monkeypatch, name) -> None:
    """After a failed attempt every retry is `just test <commit> force "<reason>"`.

    That retry runs the same complete graph, so when it passes it is the proof
    the release needs; refusing it would leave a failed commit unreleasable.
    """
    argv = ("capsem-gate", "candidate", str(COMMIT), "force", "approved retry")
    _record(config, monkeypatch, argv=argv)

    assert _execute(_release(config, name), monkeypatch) == ["prefix"]


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_a_failed_local_test_is_not_proof(config, monkeypatch, name) -> None:
    """Failed and interrupted attempts are spending history, never evidence."""
    _record(config, monkeypatch, status=FAILED)

    _refused(_release(config, name), monkeypatch, COMMIT)


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_force_does_not_waive_the_local_test(config, monkeypatch, name) -> None:
    """`--force` excuses a dirty outer checkout, never a missing proof."""
    _refused(_release(config, name, force="true"), monkeypatch, COMMIT)


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_the_unattended_nightly_scheduler_is_not_blocked(config, monkeypatch, name) -> None:
    """The daily scheduler runs on a fresh hosted runner that has no journal.

    Its lanes qualify themselves before publishing, and it cannot run a local
    `just test`, so the channels it drives are declared in
    `[release].unattended_channels` and consume no journal.
    """
    assert "nightly" in config.release.unattended_channels
    command = _release(config, name, channel="nightly")

    assert command.qualification_policy is QualificationPolicy.NONE
    assert _execute(command, monkeypatch) == ["prefix"]


@pytest.mark.parametrize("name", sorted(RELEASES))
@pytest.mark.parametrize("channel", ["stable", "corp"])
def test_every_operator_channel_requires_the_journal(config, name, channel) -> None:
    command = _release(config, name, channel=channel)

    assert command.qualification_policy is QualificationPolicy.REQUIRE
    assert set(config.release.unattended_channels) < set(config.package.channels)


# ---------------------------------------------------------------------------
# Source identity is the tree: branch proof, merge-commit release
# ---------------------------------------------------------------------------


def _git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, check=True, capture_output=True, text=True
    ).stdout.strip()


def _head(root: Path) -> SourceCommit:
    return SourceCommit(_git(root, "rev-parse", "HEAD"))


@pytest.fixture
def history(config, monkeypatch: pytest.MonkeyPatch) -> dict[str, SourceCommit]:
    """A branch head tested locally, then merged into main two ways.

    `tested` is the branch head `just test` proved. `merged` is the merge
    commit GitHub's button makes: a new SHA, the same tree. `diverged` is a
    later merge that brought in another change, so its tree differs.
    """
    # The real guard, not the fixture's stub: a release must still be on main.
    monkeypatch.setattr(qualificationflow, "require_local_main", sourcecommit.require_local_main)
    root = config.root
    _git(root, "config", "user.email", "gate@example.com")
    _git(root, "config", "user.name", "Gate")
    _git(root, "config", "commit.gpgsign", "false")
    (root / "product.txt").write_text("base\n", encoding="utf-8")
    _git(root, "add", "product.txt")
    _git(root, "commit", "-qm", "base")
    _git(root, "checkout", "-qb", "fix")
    (root / "product.txt").write_text("fixed\n", encoding="utf-8")
    _git(root, "commit", "-qam", "fix")
    tested = _head(root)
    _git(root, "checkout", "-qb", "other", "main")
    (root / "other.txt").write_text("unrelated\n", encoding="utf-8")
    _git(root, "add", "other.txt")
    _git(root, "commit", "-qm", "other")
    _git(root, "checkout", "-q", "main")
    _git(root, "merge", "-q", "--no-ff", "-m", "merge fix", "fix")
    merged = _head(root)
    _git(root, "merge", "-q", "--no-ff", "-m", "merge other", "other")
    diverged = _head(root)
    return {"tested": tested, "merged": merged, "diverged": diverged}


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_a_fast_forwarded_main_releases_the_tested_commit(
    config, history, monkeypatch, name
) -> None:
    """Supported way (a): main fast-forwards to exactly the commit tested."""
    _git(config.root, "checkout", "-q", "fix")
    _git(config.root, "branch", "-f", "main", history["tested"])
    _record(config, monkeypatch, commit=history["tested"])

    assert _execute(_release(config, name, commit=history["tested"]), monkeypatch) == ["prefix"]


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_a_merge_commit_with_the_tested_tree_reuses_the_branch_proof(
    config, history, monkeypatch, name
) -> None:
    """Supported way (b): the PR merge commit's tree equals the tested tree."""
    assert history["merged"] != history["tested"]
    _record(config, monkeypatch, commit=history["tested"])

    assert _execute(_release(config, name, commit=history["merged"]), monkeypatch) == ["prefix"]
    found = qualificationevidence.require_complete(config, history["merged"])
    assert history["tested"] in found.reference.run_log


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_a_merge_whose_tree_differs_from_the_tested_one_is_refused(
    config, history, monkeypatch, name
) -> None:
    """Anything a merge brought in changes the tree, and the proof with it."""
    _record(config, monkeypatch, commit=history["tested"])

    _refused(_release(config, name, commit=history["diverged"]), monkeypatch, history["diverged"])


def test_just_test_accepts_a_branch_head_before_it_reaches_main(config, history) -> None:
    """The proof comes first, on the branch; main is a release requirement."""
    _git(config.root, "checkout", "-qb", "next", history["tested"])
    (config.root / "product.txt").write_text("next\n", encoding="utf-8")
    _git(config.root, "commit", "-qam", "next")
    branch_head = _head(config.root)

    sourcecommit.require_local_branch(config.root, branch_head)
    with pytest.raises(GateError, match="local main"):
        sourcecommit.require_local_main(config.root, branch_head)
    dangling = SourceCommit(_git(config.root, "commit-tree", "HEAD^{tree}", "-m", "on no branch"))
    with pytest.raises(GateError, match="not on any local branch"):
        sourcecommit.require_local_branch(config.root, dangling)
