"""Every explicitly selected source contract must still name a real test file."""

from pathlib import Path

from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[2]
SOURCE_CONTRACT_RATIONALE = (
    "Source-contract membership in config/gate.toml is an explicit test selection. "
    "A stale path left after benchmark deferral made the distributed release "
    "suite collect zero tests. Collecting the enclosing directories cannot "
    "detect a deleted listed file; validate every member in the early Citadel phase."
)


def missing_contracts(root: Path, paths: tuple[str, ...]) -> list[str]:
    return [path for path in paths if not (root / path).is_file()]


def test_source_contract_inventory_names_existing_files() -> None:
    missing = missing_contracts(ROOT, gate_config.load(ROOT).suites.source_contract)
    assert not missing, SOURCE_CONTRACT_RATIONALE + f"\nMissing test files: {missing}"


def test_missing_files_and_directories_are_rejected(tmp_path: Path) -> None:
    (tmp_path / "present.py").write_text("def test_present(): pass\n")
    (tmp_path / "directory.py").mkdir()
    assert missing_contracts(tmp_path, ("present.py", "deleted.py", "directory.py")) == [
        "deleted.py", "directory.py",
    ]
