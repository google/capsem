"""The cache CLI exposes one typed inventory and mutation surface."""

import json
from pathlib import Path

from capsem_builder.cache.cli import main
from click.testing import CliRunner


def repository(tmp_path: Path) -> Path:
    config = tmp_path / "config"
    config.mkdir(parents=True)
    config.joinpath("cache.toml").write_text(
        """
version = 1
root = "cache"
authority_environment = "CAPSEM_TEST_CACHE_AUTHORITY"
[stages.objects]
description = "immutable test objects"
scope = "disk"
path = "target/objects"
warm_size_bytes = 2
max_size_bytes = 3
prune_strategy = "lru"
maximum_age_hours = 72
""".strip(),
        encoding="utf-8",
    )
    return tmp_path


def invoke(root: Path, *arguments: str):
    return CliRunner().invoke(main, ["--repository", str(root), *arguments])


def test_stats_json_reports_usage_and_contract(tmp_path: Path) -> None:
    root = repository(tmp_path)
    entry = root / "cache/target/objects/one"
    entry.mkdir(parents=True)
    (entry / "payload").write_bytes(b"abc")

    result = invoke(root, "stats", "--json")

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["caches"][0] == {
        "cache_id": "objects",
        "description": "immutable test objects",
        "scope": "disk",
        "current_size_bytes": 3,
        "warm_size_bytes": 2,
        "max_size_bytes": 3,
        "prune_strategy": "lru",
        "state": "above-warm",
    }


def test_policy_source_is_independent_from_cache_storage(tmp_path: Path) -> None:
    policy_root = repository(tmp_path / "source")
    storage_root = tmp_path / "storage"
    entry = storage_root / "cache/target/objects/one"
    entry.mkdir(parents=True)
    (entry / "payload").write_bytes(b"shared")

    result = CliRunner().invoke(
        main,
        [
            "--repository",
            str(storage_root),
            "--policy-repository",
            str(policy_root),
            "stats",
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["caches"][0]["current_size_bytes"] == len(b"shared")


def test_prune_previews_then_applies_only_with_flag(tmp_path: Path) -> None:
    root = repository(tmp_path)
    entry = root / "cache/target/objects/one"
    entry.mkdir(parents=True)
    (entry / "payload").write_bytes(b"abcd")

    preview = invoke(root, "prune", "objects")
    assert preview.exit_code == 0, preview.output
    assert entry.exists() and "PREVIEW" in preview.output

    applied = invoke(root, "prune", "objects", "--apply")
    assert applied.exit_code == 0, applied.output
    assert not entry.exists()


def test_clean_all_requires_a_reason_when_applied(tmp_path: Path) -> None:
    result = invoke(repository(tmp_path), "clean", "all", "--apply")

    assert result.exit_code != 0
    assert "--reason" in result.output


def test_dispatch_preserves_just_argument_boundaries(tmp_path: Path) -> None:
    result = invoke(repository(tmp_path), "dispatch", "stats", "--json")

    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["caches"][0]["cache_id"] == "objects"


def test_dispatch_keeps_a_multiword_reason_as_one_value(tmp_path: Path) -> None:
    result = invoke(repository(tmp_path), "dispatch", "prune", "--apply", "--reason", "two words")

    assert result.exit_code == 0, result.output
    assert "APPLIED" in result.output


def test_verify_rejects_unclassified_cache_paths(tmp_path: Path) -> None:
    root = repository(tmp_path)
    (root / "cache/target/stray").mkdir(parents=True)

    result = invoke(root, "verify")

    assert result.exit_code != 0
    assert "unclassified cache paths: target/stray" in result.output


def test_contract_hides_backend_configuration(tmp_path: Path) -> None:
    result = invoke(repository(tmp_path), "contract", "objects")

    assert result.exit_code == 0, result.output
    assert json.loads(result.output) == {
        "description": "immutable test objects",
        "scope": "disk",
        "max_size_bytes": 3,
        "warm_size_bytes": 2,
        "prune_strategy": "lru",
    }


def test_stats_reports_maximum_violation(tmp_path: Path) -> None:
    root = repository(tmp_path)
    payload = root / "cache/target/objects/one/payload"
    payload.parent.mkdir(parents=True)
    payload.write_bytes(b"over-the-max")

    result = invoke(root, "stats", "--json")

    assert result.exit_code == 0, result.output
    report = json.loads(result.output)
    assert not report["healthy"]
    assert report["caches"][0]["state"] == "above-max"
