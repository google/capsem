"""Test helpers look for host binaries where Cargo put them.

The bounded diagnostics wrapper exports `CARGO_TARGET_DIR` under the shared
cache authority -- the git common checkout -- so a linked worktree's build
lands outside its own `cache/target/cargo`. `host_bin_root` still read the
checkout-relative default, and the gateway suite quietly ran an older binary
than the source it claimed to test: a tunnel fix looked broken until the
binary was rebuilt by hand.
"""

from __future__ import annotations

from pathlib import Path

from helpers.constants import PROJECT_ROOT, host_bin_root

HOST_BINARY_ROOT_RATIONALE = (
    "helpers must resolve host binaries from the same variables Cargo reads; "
    "a checkout-relative default tests a binary nobody just built"
)


def test_release_bin_dir_wins(tmp_path: Path) -> None:
    release = tmp_path / "release"
    environment = {"CAPSEM_RELEASE_BIN_DIR": str(release), "CARGO_TARGET_DIR": str(tmp_path / "cargo")}
    assert host_bin_root(environment) == release, HOST_BINARY_ROOT_RATIONALE


def test_cargo_target_dir_is_where_debug_binaries_are(tmp_path: Path) -> None:
    environment = {"CARGO_TARGET_DIR": str(tmp_path / "cargo")}
    assert host_bin_root(environment) == tmp_path / "cargo" / "debug", HOST_BINARY_ROOT_RATIONALE


def test_without_either_the_checkout_target_is_used() -> None:
    assert host_bin_root({}) == PROJECT_ROOT / "cache" / "target" / "cargo" / "debug", HOST_BINARY_ROOT_RATIONALE


def test_empty_variables_are_unset() -> None:
    environment = {"CAPSEM_RELEASE_BIN_DIR": "", "CARGO_TARGET_DIR": ""}
    assert host_bin_root(environment) == PROJECT_ROOT / "cache" / "target" / "cargo" / "debug", HOST_BINARY_ROOT_RATIONALE
