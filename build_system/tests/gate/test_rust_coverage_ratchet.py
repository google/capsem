"""The Rust coverage average cannot hide an untested workspace crate."""

from __future__ import annotations

import importlib.util
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "scripts"
    / "test"
    / "rust-coverage-ratchet.py"
)
SPEC = importlib.util.spec_from_file_location("rust_coverage_ratchet", SCRIPT)
assert SPEC and SPEC.loader
RATCHET = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RATCHET)


def _manifest(root: Path, directory: str, package: str) -> None:
    crate = root / "native" / directory
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "{package}"\nversion = "0.1.0"\n'
    )


def test_workspace_inventory_uses_package_names_and_configured_root(
    tmp_path: Path,
) -> None:
    _manifest(tmp_path, "shell", "product-cli")
    _manifest(tmp_path, "engine", "product-core")

    assert RATCHET.workspace_crates(tmp_path, Path("native")) == {
        "engine": "product-core",
        "shell": "product-cli",
    }


def test_lcov_groups_unique_lines_by_owning_crate(tmp_path: Path) -> None:
    _manifest(tmp_path, "engine", "product-core")
    report = tmp_path / "coverage.lcov"
    source = tmp_path / "native" / "engine" / "src" / "lib.rs"
    report.write_text(
        f"SF:{source}\nDA:10,0\nDA:11,2\nend_of_record\n"
        f"SF:{source}\nDA:10,3\nDA:11,0\nend_of_record\n"
        "SF:/external/vendor.rs\nDA:1,1\nend_of_record\n"
    )

    measured = RATCHET.lcov_by_crate(
        report,
        tmp_path,
        Path("native"),
        {"engine": "product-core"},
    )

    assert measured == {"product-core": RATCHET.Coverage(hit=2, found=2)}


def test_floor_inventory_must_match_every_workspace_crate() -> None:
    problems = RATCHET.violations(
        {"product-core": RATCHET.Coverage(hit=8, found=10)},
        {"product-core", "product-cli"},
        {"product-core": 75.0, "retired": 10.0},
        5.0,
    )

    assert any("missing workspace crates: ['product-cli']" in item for item in problems)
    assert any("stale workspace crates: ['retired']" in item for item in problems)
    assert any("LCOV report omitted workspace crates: ['product-cli']" in item for item in problems)


def test_a_crate_below_its_floor_fails_independently_of_the_average() -> None:
    problems = RATCHET.violations(
        {
            "large": RATCHET.Coverage(hit=990, found=1000),
            "small": RATCHET.Coverage(hit=4, found=10),
        },
        {"large", "small"},
        {"large": 95.0, "small": 50.0},
        5.0,
    )

    assert not any(item.startswith("large:") for item in problems)
    assert any("small: 40.00% is below its 50.00% floor" in item for item in problems)


def test_improvement_past_headroom_requires_the_floor_to_ratchet() -> None:
    problems = RATCHET.violations(
        {"product-core": RATCHET.Coverage(hit=90, found=100)},
        {"product-core"},
        {"product-core": 80.0},
        3.0,
    )

    assert problems == [
        "product-core: 90.00% leaves 10.00 points above its 80.00% floor; "
        "raise the floor to at least 87.00% in the same change"
    ]


def test_coverage_inside_the_platform_variation_band_passes() -> None:
    assert (
        RATCHET.violations(
            {"product-core": RATCHET.Coverage(hit=82, found=100)},
            {"product-core"},
            {"product-core": 80.0},
            3.0,
        )
        == []
    )
