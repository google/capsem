"""Git impact includes committed, working-tree, and untracked source."""

import subprocess
from pathlib import Path

from capsem_builder.cache.gitimpact import inspect_git


def git(root: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", *arguments],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def test_working_source_impact_includes_every_git_source_shape(tmp_path: Path) -> None:
    git(tmp_path, "init", "-q", "-b", "main")
    git(tmp_path, "config", "user.name", "Cache Test")
    git(tmp_path, "config", "user.email", "cache@example.test")
    tracked = tmp_path / "config/settings/settings.toml"
    tracked.parent.mkdir(parents=True)
    tracked.write_text("one", encoding="utf-8")
    git(tmp_path, "add", "config/settings/settings.toml")
    git(tmp_path, "commit", "-qm", "baseline")
    baseline = git(tmp_path, "rev-parse", "HEAD")
    tracked.write_text("two", encoding="utf-8")
    git(tmp_path, "commit", "-am", "settings")
    (tmp_path / "README.md").write_text("untracked", encoding="utf-8")

    impact = inspect_git(tmp_path, baseline, None)

    assert impact.ancestor is True
    assert impact.commits == 1
    assert impact.paths == ("README.md", "config/settings/settings.toml")
