"""Regression tests for the commit identity embedded by ``crates/capsem/build.rs``."""

from __future__ import annotations

import os
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
BUILD_SCRIPT = REPO_ROOT / "crates" / "capsem" / "build.rs"
PROVENANCE_CHECK = REPO_ROOT / "scripts" / "check-build-provenance.sh"


def test_release_profile_keeps_codegen_parallel_without_weakening_artifacts() -> None:
    release = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())["profile"]["release"]

    assert "codegen-units" not in release, (
        "the Cargo release default partitions codegen for parallel execution; "
        "forcing one unit serialized the largest package builds on 16-core hosts"
    )
    assert release["lto"] == "thin"
    assert release["strip"] == "symbols"


def _run(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, (
        f"{' '.join(command)} failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
    return result


def _commit(repo: Path, filename: str, contents: str, message: str) -> str:
    (repo / filename).write_text(contents)
    _run(["git", "add", filename], cwd=repo)
    _run(["git", "commit", "--quiet", "-m", message], cwd=repo)
    return _run(["git", "rev-parse", "--short", "HEAD"], cwd=repo).stdout.strip()


def _make_test_repository(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    crate = repo / "crates" / "capsem"
    (crate / "src").mkdir(parents=True)
    shutil.copy2(BUILD_SCRIPT, crate / "build.rs")
    (crate / "Cargo.toml").write_text(
        """
[package]
name = "build-provenance-fixture"
version = "0.0.0"
edition = "2021"
build = "build.rs"
""".lstrip()
    )
    (crate / "src" / "main.rs").write_text(
        'fn main() { println!("{}", env!("CAPSEM_BUILD_HASH")); }\n'
    )

    _run(["git", "init", "--quiet", "--initial-branch=main"], cwd=repo)
    _run(["git", "config", "user.name", "Capsem Test"], cwd=repo)
    _run(["git", "config", "user.email", "test@capsem.invalid"], cwd=repo)
    _run(["git", "config", "commit.gpgsign", "false"], cwd=repo)
    _run(["git", "add", "."], cwd=repo)
    _run(["git", "commit", "--quiet", "-m", "initial"], cwd=repo)
    return repo


def _embedded_hash(repo: Path, target_dir: Path) -> str:
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target_dir)
    return _run(
        [
            "cargo",
            "run",
            "--quiet",
            "--manifest-path",
            "crates/capsem/Cargo.toml",
        ],
        cwd=repo,
        env=env,
    ).stdout.strip()


@pytest.mark.parametrize("linked_worktree", [False, True])
def test_embedded_hash_follows_head_across_cached_builds(
    tmp_path: Path,
    linked_worktree: bool,
) -> None:
    """A new commit must invalidate build-script output in repos and worktrees."""
    repo = _make_test_repository(tmp_path)
    checkout = repo
    if linked_worktree:
        checkout = tmp_path / "linked"
        _run(
            ["git", "worktree", "add", "--quiet", "-b", "release-test", str(checkout)],
            cwd=repo,
        )

    target_dir = tmp_path / f"target-{linked_worktree}"
    before_sha = _run(["git", "rev-parse", "--short", "HEAD"], cwd=checkout).stdout.strip()
    before_hash = _embedded_hash(checkout, target_dir)
    assert before_hash.startswith(f"{before_sha}.")

    after_sha = _commit(
        checkout,
        "release-state",
        "candidate two\n",
        "advance release source",
    )
    after_hash = _embedded_hash(checkout, target_dir)

    assert after_sha != before_sha
    assert after_hash.startswith(f"{after_sha}."), (
        "cached package build retained stale source provenance: "
        f"expected {after_sha}, got {after_hash}"
    )


def _fake_capsem(tmp_path: Path, version_output: str) -> Path:
    binary = tmp_path / "capsem"
    binary.write_text(f"#!/bin/sh\nprintf '%s\\n' '{version_output}'\n")
    binary.chmod(0o755)
    return binary


def test_package_provenance_check_accepts_exact_revision(tmp_path: Path) -> None:
    binary = _fake_capsem(
        tmp_path,
        "capsem 1.6.1 (build abc1234.1785231406 ts=dev)",
    )

    result = _run(
        ["bash", str(PROVENANCE_CHECK), str(binary), "abc1234"],
        cwd=REPO_ROOT,
    )

    assert "Exact build provenance verified: abc1234" in result.stdout


def test_package_provenance_check_rejects_stale_revision(tmp_path: Path) -> None:
    binary = _fake_capsem(
        tmp_path,
        "capsem 1.6.1 (build e1de77a.1785231406 ts=dev)",
    )

    result = subprocess.run(
        ["bash", str(PROVENANCE_CHECK), str(binary), "9070367"],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode != 0
    assert "does not embed exact source revision 9070367" in result.stderr


def test_every_package_builder_enforces_exact_provenance() -> None:
    macos_builder = (REPO_ROOT / "scripts" / "build-test-macos-package.sh").read_text()
    linux_builder = (REPO_ROOT / "scripts" / "build-linux-package.sh").read_text()
    release_workflow = (REPO_ROOT / ".github" / "workflows" / "release.yaml").read_text()

    assert 'bash scripts/check-build-provenance.sh "$ROOT/target/release/capsem"' in macos_builder
    # Was asserted against the justfile, where this lived as an escaped
    # fragment of a `docker run ... bash -c` argument.
    assert (
        'bash scripts/check-build-provenance.sh "/cargo-target/$RUST_TARGET/release/capsem"'
        in linux_builder
    )
    assert (
        release_workflow.count("bash scripts/check-build-provenance.sh target/release/capsem") == 1
    ), "the macOS publication builder must reject stale provenance directly"
    assert "uv run capsem-gate cross-compile" in release_workflow
