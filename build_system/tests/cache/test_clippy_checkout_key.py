"""Clippy output in the shared Cargo target is keyed by checkout.

`cargo clippy` forces one shared clippy-driver path as the workspace wrapper,
so checkouts sharing a target lost the key `rustc-workspace-wrapper.sh` gives
`cargo check`. A checkout with older sources then took another checkout's newer
clippy artifact as fresh: its own lints went unreported, or the other tree's
were. `clippyrun` runs clippy through a checkout-local wrapper instead.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest
from capsem_builder.gate import clippyrun
from capsem_builder.gate.tools.ci import run_bounded_command

ROOT = Path(__file__).resolve().parents[3]
WRAPPER = tomllib.loads((ROOT / "config/gate.toml").read_text())["toolchain"]["clippy_workspace_wrapper"]
LINTED = "pub fn size() -> usize { let values = vec![1]; values.len() }\n"
CLEAN = "pub fn size() -> usize { 1 }\n"


def _tree(root: Path, source: str) -> Path:
    (root / "src").mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname="clippy-key"\nversion="0.1.0"\nedition="2021"\n')
    (root / "src/lib.rs").write_text(source)
    shutil.copy2(ROOT / "rust-toolchain.toml", root / "rust-toolchain.toml")
    (root / WRAPPER).parent.mkdir(parents=True)
    shutil.copy2(ROOT / WRAPPER, root / WRAPPER)
    # One ancient mtime everywhere: nothing here can pass on timestamps.
    for path in root.rglob("*"):
        if path.is_file():
            os.utime(path, (1_000_000_000, 1_000_000_000))
    return root


def _clippy(tree: Path, target: Path) -> subprocess.CompletedProcess[str]:
    argv, keyed = clippyrun.invocation(WRAPPER, ["--offline", "--message-format=short"], ["-D", "warnings"])
    unwrapped = ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER")
    environment: dict[str, str] = {
        **{key: value for key, value in os.environ.items() if key not in unwrapped},
        "CARGO_TARGET_DIR": str(target),
        **keyed,
    }
    return subprocess.run(argv, cwd=tree, env=environment, capture_output=True, text=True, timeout=120)


@pytest.mark.skipif(shutil.which("cargo") is None, reason="needs the Rust toolchain")
def test_each_checkout_gets_its_own_clippy_verdict_from_a_shared_target(tmp_path: Path) -> None:
    clean = _tree(tmp_path / "clean", CLEAN)
    linted = _tree(tmp_path / "linted", LINTED)
    target = tmp_path / "cache/target/cargo"

    first = _clippy(clean, target)
    assert first.returncode == 0, first.stderr
    # The linted tree is no newer than the clean tree's artifact; only the key
    # keeps it from being taken as fresh.
    second = _clippy(linted, target)
    assert second.returncode != 0 and "useless use of `vec!`" in second.stderr, second.stderr
    # Returning to a checkout reuses its own work rather than rechecking.
    third = _clippy(clean, target)
    assert third.returncode == 0, third.stderr
    assert "Checking clippy-key" not in third.stderr, third.stderr


def test_cargo_clippy_commands_become_the_keyed_form() -> None:
    wrapper = "/checkout/build_system/scripts/build/clippy-workspace-wrapper.sh"
    translated = clippyrun.from_cargo_clippy(
        ["cargo", "+1.97.1", "clippy", "-p", "capsem-service", "--tests", "--", "-D", "warnings"], wrapper
    )
    assert translated is not None
    argv, keyed = translated
    assert argv == [
        "cargo", "+1.97.1", "check", "--config", f'build.rustc-workspace-wrapper="{wrapper}"',
        "-p", "capsem-service", "--tests",
    ]
    assert keyed == {"CLIPPY_ARGS": "-D__CLIPPY_HACKERY__warnings__CLIPPY_HACKERY__"}
    assert clippyrun.from_cargo_clippy(["cargo", "clippy"], wrapper) == (
        ["cargo", "check", "--config", f'build.rustc-workspace-wrapper="{wrapper}"'],
        {"CLIPPY_ARGS": ""},
    )
    for untouched in (["cargo", "check"], ["cargo", "clippy", "--fix"], ["just", "clippy"], []):
        assert clippyrun.from_cargo_clippy(untouched, wrapper) is None


def test_bounded_launcher_keys_clippy_by_its_own_checkout() -> None:
    command, keyed = run_bounded_command._keyed_clippy(["cargo", "clippy", "--workspace"], ROOT)
    assert command[:4] == ["cargo", "check", "--config", f'build.rustc-workspace-wrapper="{ROOT / WRAPPER}"']
    assert "CLIPPY_ARGS" in keyed
    assert run_bounded_command._keyed_clippy(["cargo", "test"], ROOT) == (["cargo", "test"], {})


def test_no_gate_step_runs_unkeyed_cargo_clippy() -> None:
    """A literal `cargo clippy` in a gate module reintroduces the shared key."""
    offenders = [
        f"{path.relative_to(ROOT)}"
        for path in sorted((ROOT / "build_system/builder").rglob("*.py"))
        if '"cargo", "clippy"' in path.read_text(encoding="utf-8")
    ]
    assert offenders == [], offenders
