"""The Rust coverage average cannot hide an untested workspace crate."""

from __future__ import annotations

import importlib.util
import json
import platform
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "test" / "rust-coverage-ratchet.py"
SPEC = importlib.util.spec_from_file_location("rust_coverage_ratchet", SCRIPT)
assert SPEC and SPEC.loader
RATCHET = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RATCHET)


@pytest.mark.parametrize("failure", ["omitted_report", "missing_floor", "regression"])
def test_sdk_member_cannot_hide_behind_core_coverage(tmp_path, failure):
    members = {"native/rust": "product-core", "sdk/rust": "capsem-sdk"}
    (tmp_path / "Cargo.toml").write_text(
        "[workspace]\nmembers=" + json.dumps(list(members)) + "\n"
    )
    for directory, name in members.items():
        crate = tmp_path / directory
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.1.0"\n')
    crates = RATCHET.workspace_crates(tmp_path, Path("Cargo.toml"))
    assert crates == members, "SDK and core may share a basename; both must be inventoried"
    report = tmp_path / "coverage.lcov"
    source = "SF:native/rust/src/lib.rs\nDA:1,1\nDA:2,1\n"
    if failure != "omitted_report":
        source += "SF:sdk/rust/src/generated/api.rs\nDA:1,1\nDA:2,0\n"
    report.write_text(source)
    measured = RATCHET.lcov_by_crate(report, tmp_path, crates)
    floors = {"product-core": 97.0, "capsem-sdk": 80.0}
    if failure == "missing_floor":
        floors.pop("capsem-sdk")
    problems = RATCHET.violations(measured, set(members.values()), floors, 3.0, 40.0)
    expected = {
        "omitted_report": "LCOV report omitted workspace crates: ['capsem-sdk']",
        "missing_floor": "coverage floors missing workspace crates: ['capsem-sdk']",
        "regression": "capsem-sdk: 50.00% is below its 80.00% floor",
    }[failure]
    assert any(expected in problem for problem in problems)


@pytest.mark.parametrize("system,hits", [("Linux", 77), ("Darwin", 82)])
def test_each_host_ratchets_its_own_compiled_coverage(tmp_path, monkeypatch, system, hits):
    _manifest(tmp_path, "engine", "product-core")
    report = tmp_path / "coverage.lcov"
    report.write_text(
        "SF:native/engine/src/lib.rs\n"
        + "".join(f"DA:{row},{int(row <= hits)}\n" for row in range(1, 101))
    )
    (tmp_path / "gate.toml").write_text(
        "[modules]\nrust_coverage_crate_minimum=40.0\n"
        "rust_coverage_ratchet_headroom=3.0\n"
        "rust_coverage_crate_floors={product-core=75.0}\n"
        "rust_coverage_platform_crate_floors={Linux={},Darwin={product-core=80.0}}\n"
    )
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(platform, "system", lambda: system)

    assert (
        RATCHET.main(
            [
                "--report",
                str(report),
                "--workspace-manifest",
                "Cargo.toml",
                "--config",
                "gate.toml",
            ]
        )
        == 0
    )


def _manifest(root: Path, directory: str, package: str) -> None:
    crate = root / "native" / directory
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(f'[package]\nname = "{package}"\nversion = "0.1.0"\n')
    members = sorted(path.parent.relative_to(root).as_posix()
                     for path in (root / "native").glob("*/Cargo.toml"))
    (root / "Cargo.toml").write_text("[workspace]\nmembers=" + json.dumps(members) + "\n")


def test_workspace_inventory_uses_package_names_and_declared_members(
    tmp_path: Path,
) -> None:
    _manifest(tmp_path, "shell", "product-cli")
    _manifest(tmp_path, "engine", "product-core")

    assert RATCHET.workspace_crates(tmp_path, Path("Cargo.toml")) == {
        "native/engine": "product-core",
        "native/shell": "product-cli",
    }


def test_workspace_globs_exclusions_and_root_package(tmp_path):
    _manifest(tmp_path, "engine", "product-core")
    _manifest(tmp_path, "excluded", "outside-workspace")
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname="root-package"\nversion="0.1.0"\n'
        '[workspace]\nmembers=["native/*"]\nexclude=["native/excluded"]\n'
    )
    crates = RATCHET.workspace_crates(tmp_path, Path("Cargo.toml"))
    assert crates == {".": "root-package", "native/engine": "product-core"}
    report = tmp_path / "coverage.lcov"
    report.write_text("SF:src/lib.rs\nDA:1,1\nSF:native/engine/src/lib.rs\nDA:1,0\n")
    assert RATCHET.lcov_by_crate(report, tmp_path, crates) == {
        "root-package": RATCHET.Coverage(hit=1, found=1),
        "product-core": RATCHET.Coverage(hit=0, found=1),
    }


def test_missing_workspace_members_fail_loudly(tmp_path):
    (tmp_path / "Cargo.toml").write_text('[workspace]\nmembers=["sdk/missing"]\n')
    with pytest.raises(ValueError, match="matches nothing"):
        RATCHET.workspace_crates(tmp_path, Path("Cargo.toml"))


@pytest.mark.parametrize(
    "overrides,system",
    [
        ({"Linux": {}, "Darwin": {"product-core": 74.99}}, "Linux"),
        ({"Linux": {}, "Darwin": {"unknown": 80.0}}, "Darwin"),
        ({"Linux": {}, "Darwin": {"product-core": 101.0}}, "Darwin"),
        ({"Linux": {}}, "Darwin"),
    ],
)
def test_platform_floors_fail_closed_without_weakening_baseline(overrides, system):
    with pytest.raises(ValueError):
        RATCHET.platform_floors({"product-core": 75.0}, overrides, system)


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
        {"native/engine": "product-core"},
    )

    assert measured == {"product-core": RATCHET.Coverage(hit=2, found=2)}


def test_floor_inventory_must_match_every_workspace_crate() -> None:
    problems = RATCHET.violations(
        {"product-core": RATCHET.Coverage(hit=8, found=10)},
        {"product-core", "product-cli"},
        {"product-core": 75.0, "retired": 10.0},
        5.0,
        40.0,
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
        40.0,
    )

    assert not any(item.startswith("large:") for item in problems)
    assert any("small: 40.00% is below its 50.00% floor" in item for item in problems)


def test_improvement_past_headroom_requires_the_floor_to_ratchet() -> None:
    problems = RATCHET.violations(
        {"product-core": RATCHET.Coverage(hit=90, found=100)},
        {"product-core"},
        {"product-core": 80.0},
        3.0,
        40.0,
    )

    assert problems == [
        "product-core: 90.00% leaves 10.00 points above its 80.00% floor; "
        "raise the floor to at least 87.00% in the same change"
    ]


def test_required_floor_rounds_up_to_a_sufficient_hundredth() -> None:
    coverage = RATCHET.Coverage(hit=78514, found=100000)

    problems = RATCHET.violations(
        {"product-core": coverage},
        {"product-core"},
        {"product-core": 75.50},
        3.0,
        40.0,
    )

    assert problems == [
        "product-core: 78.51% leaves 3.01 points above its 75.50% floor; "
        "raise the floor to at least 75.52% in the same change"
    ]
    assert (
        RATCHET.violations(
            {"product-core": coverage},
            {"product-core"},
            {"product-core": 75.52},
            3.0,
            40.0,
        )
        == []
    )


def test_coverage_inside_the_platform_variation_band_passes() -> None:
    assert (
        RATCHET.violations(
            {"product-core": RATCHET.Coverage(hit=82, found=100)},
            {"product-core"},
            {"product-core": 80.0},
            3.0,
            40.0,
        )
        == []
    )


def test_no_crate_floor_can_fall_below_the_workspace_minimum() -> None:
    problems = RATCHET.violations(
        {"product-core": RATCHET.Coverage(hit=90, found=100)},
        {"product-core"},
        {"product-core": 39.99},
        60.0,
        40.0,
    )

    assert problems == ["product-core: 39.99% floor is below the workspace crate minimum of 40.00%"]


def test_invalid_workspace_minimum_fails_closed() -> None:
    problems = RATCHET.violations(
        {"product-core": RATCHET.Coverage(hit=90, found=100)},
        {"product-core"},
        {"product-core": 80.0},
        20.0,
        101.0,
    )

    assert "invalid minimum crate coverage floor 101.00%" in problems
