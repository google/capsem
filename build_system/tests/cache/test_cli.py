"""The cache CLI exposes read-only inventory and explicit mutation."""

import json
from pathlib import Path

from capsem_builder.cache.cli import main
from click.testing import CliRunner


def repository(tmp_path: Path) -> Path:
    config = tmp_path / "config"
    config.mkdir()
    config.joinpath("cache.toml").write_text(
        """
version = 1
root = "cache"
minimum_free_bytes = 1
[stages.objects]
path = "target/objects"
warning_bytes = 1
soft_bytes = 2
hard_bytes = 3
prune = "lru"
maximum_age_hours = 72
""".strip(),
        encoding="utf-8",
    )
    return tmp_path


def invoke(root: Path, *arguments: str):
    return CliRunner().invoke(main, ["--repository", str(root), *arguments])


def test_status_json_reports_stage_usage(tmp_path: Path) -> None:
    root = repository(tmp_path)
    entry = root / "cache/target/objects/one"
    entry.mkdir(parents=True)
    (entry / "payload").write_bytes(b"abc")

    result = invoke(root, "status", "--json")

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["stages"][0]["stage_id"] == "objects"
    assert payload["stages"][0]["logical_bytes"] == 3


def test_prune_previews_then_applies_only_with_flag(tmp_path: Path) -> None:
    root = repository(tmp_path)
    entry = root / "cache/target/objects/one"
    entry.mkdir(parents=True)
    (entry / "payload").write_bytes(b"abc")

    preview = invoke(root, "prune")
    assert preview.exit_code == 0, preview.output
    assert entry.exists()
    assert "preview" in preview.output.lower()

    applied = invoke(root, "prune", "--apply")
    assert applied.exit_code == 0, applied.output
    assert not entry.exists()


def test_clean_all_requires_a_reason_when_applied(tmp_path: Path) -> None:
    root = repository(tmp_path)

    result = invoke(root, "clean", "all", "--apply")

    assert result.exit_code != 0
    assert "--reason" in result.output


def test_dispatch_safely_parses_the_joined_just_command(tmp_path: Path) -> None:
    root = repository(tmp_path)

    result = invoke(root, "dispatch", "status --json")

    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["stages"][0]["stage_id"] == "objects"
