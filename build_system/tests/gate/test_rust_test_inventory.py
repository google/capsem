from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate.rustinventory import (
    InventoryMismatch,
    RustTarget,
    RustTestInventory,
)

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "tests" / "fixtures" / "rust-test-inventory" / "Cargo.toml"


def _cargo_env(target_dir: Path) -> dict[str, str]:
    return {**os.environ, "CARGO_TARGET_DIR": str(target_dir)}


def _json_output(*argv: str, target_dir: Path) -> object:
    result = subprocess.run(
        argv,
        cwd=ROOT,
        env=_cargo_env(target_dir),
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        pytest.fail(
            f"{shlex.join(argv)} failed ({result.returncode}):\n{result.stderr}",
            pytrace=False,
        )
    return json.loads(result.stdout)


def _inventories(
    target_dir: Path, *nextest_args: str
) -> tuple[RustTestInventory, RustTestInventory]:
    metadata = _json_output(
        "cargo",
        "metadata",
        "--format-version",
        "1",
        "--no-deps",
        "--manifest-path",
        str(FIXTURE),
        target_dir=target_dir,
    )
    nextest = _json_output(
        "cargo",
        "nextest",
        "list",
        "--manifest-path",
        str(FIXTURE),
        *nextest_args,
        "--message-format",
        "json",
        target_dir=target_dir,
    )
    return (
        RustTestInventory.from_cargo_metadata(metadata),
        RustTestInventory.from_nextest_list(nextest),
    )


@pytest.fixture(scope="module")
def cargo_target_dir(tmp_path_factory: pytest.TempPathFactory) -> Path:
    return tmp_path_factory.mktemp("rust-test-inventory") / "cache" / "target"


def test_cargo_and_nextest_agree_on_every_native_target(cargo_target_dir: Path) -> None:
    cargo, nextest = _inventories(cargo_target_dir)

    expected = frozenset(
        {
            RustTarget(
                package="rust-test-inventory-fixture",
                name="rust_test_inventory_fixture",
                kind="lib",
            ),
            RustTarget(
                package="rust-test-inventory-fixture",
                name="fixture_bin",
                kind="bin",
            ),
            RustTarget(
                package="rust-test-inventory-fixture",
                name="integration",
                kind="test",
            ),
        }
    )

    assert cargo.native == expected
    assert nextest.native == expected
    cargo.require_same_native_targets(nextest)


def test_doctests_are_owned_separately_from_nextest(cargo_target_dir: Path) -> None:
    cargo, nextest = _inventories(cargo_target_dir)

    library = RustTarget(
        package="rust-test-inventory-fixture",
        name="rust_test_inventory_fixture",
        kind="lib",
    )
    assert cargo.doctest == frozenset({library})
    assert nextest.doctest == frozenset()

    subprocess.run(
        ["cargo", "test", "--doc", "--manifest-path", str(FIXTURE)],
        cwd=ROOT,
        env=_cargo_env(cargo_target_dir),
        check=True,
        capture_output=True,
        text=True,
    )


def test_examples_are_not_native_correctness_targets(cargo_target_dir: Path) -> None:
    cargo, nextest = _inventories(cargo_target_dir)

    assert all(target.kind != "example" for target in cargo.native)
    assert all(target.kind != "example" for target in nextest.native)


def test_missing_target_fails_with_the_exact_identity(cargo_target_dir: Path) -> None:
    cargo, nextest = _inventories(cargo_target_dir)
    integration = next(target for target in nextest.native if target.kind == "test")
    incomplete = nextest.model_copy(update={"native": nextest.native - {integration}})

    with pytest.raises(InventoryMismatch, match="missing from Nextest") as failure:
        cargo.require_same_native_targets(incomplete)

    assert integration.render() in str(failure.value)


def test_bins_only_selection_is_mechanically_rejected(cargo_target_dir: Path) -> None:
    cargo, bins_only = _inventories(cargo_target_dir, "--bins")

    with pytest.raises(InventoryMismatch, match="missing from Nextest") as failure:
        cargo.require_same_native_targets(bins_only)

    message = str(failure.value)
    assert "rust-test-inventory-fixture:lib/rust_test_inventory_fixture" in message
    assert "rust-test-inventory-fixture:test/integration" in message


def test_host_platform_sentinel_is_listed(cargo_target_dir: Path) -> None:
    listing = subprocess.run(
        [
            "cargo",
            "nextest",
            "list",
            "--manifest-path",
            str(FIXTURE),
            "--message-format",
            "json",
        ],
        cwd=ROOT,
        env=_cargo_env(cargo_target_dir),
        check=True,
        capture_output=True,
        text=True,
    )
    suites = json.loads(listing.stdout)["rust-suites"]
    cases = {
        case
        for suite in suites.values()
        if suite["kind"] == "lib"
        for case in suite["testcases"]
    }

    host = "macos" if sys.platform == "darwin" else sys.platform
    assert f"tests::{host}_sentinel" in cases
