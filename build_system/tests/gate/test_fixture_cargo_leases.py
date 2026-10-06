"""Fixture Cargo routing preserves its probe while taking the machine lease.

Subprocess recording here tests the actual fixture's launcher argv, not Rust
correctness. Native freshness/inventory/verdict assertions remain in their
existing owners and must still run against the real toolchain.
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import time
from pathlib import Path

import pytest
from capsem_builder.gate import boundedlease, config
from helpers.bounded import WRAPPER, bounded
from test_bounded_machine_lease import _environment, _gate_holds_the_machine

ROOT = Path(__file__).resolve().parents[3]
REHEARSAL = "build_system/scripts/release/rehearse-asset-channel-staging.sh"


@pytest.mark.parametrize("prefix", [["bash"], ["/bin/bash"], ["env", "BUILD_PROFILE=probe", "bash"]])
def test_asset_rehearsal_takes_the_lease_before_preparing_its_fixture(prefix: list[str]) -> None:
    command = [*prefix, REHEARSAL, "staging", "1.0.2", "fixture", "dist", "evidence"]
    assert boundedlease.machine_work(command, config.load(ROOT).locks.bounded), (
        "the rehearsal's outer timeout must start after its Cargo lease wait"
    )


def test_rehearsal_script_wait_does_not_spend_its_command_budget(tmp_path: Path) -> None:
    """Real wrapper and kernel lease; shim observes script startup, not artifact correctness."""
    record = tmp_path / "script-started"
    environment = _environment(tmp_path)
    bash = tmp_path / "bin/bash"
    bash.write_text(f"#!/bin/sh\nprintf '%s' \"$CAPSEM_GATE_RUN\" > '{record}'\n")
    bash.chmod(0o755)
    with _gate_holds_the_machine(tmp_path):
        waiting = subprocess.Popen(
            bounded(["bash", REHEARSAL, "staging", "1.0.2", "fixture", "dist", "evidence"], 1),
            env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            assert waiting.stderr is not None
            assert "waiting:" in waiting.stderr.readline()
            time.sleep(1.25)
            assert waiting.poll() is None, "a lock wait consumed the script's execution timeout"
            assert not record.exists(), "fixture preparation started before acquiring the lease"
        except BaseException:
            waiting.terminate()
            waiting.communicate(timeout=20)
            raise
    _, error = waiting.communicate(timeout=30)
    assert waiting.returncode == 0, error
    assert record.read_text().startswith("bounded: bash " + REHEARSAL)


@pytest.mark.parametrize("owner", ["inventory", "clippy", "provenance"])
def test_actual_fixture_cargo_invocations_take_the_bounded_route(
    owner: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    paths = {
        "inventory": "build_system/tests/gate/test_rust_test_inventory.py",
        "clippy": "build_system/tests/cache/test_clippy_checkout_key.py",
        "provenance": "tests/test_build_provenance.py",
    }
    spec = importlib.util.spec_from_file_location(f"fixture_{owner}", ROOT / paths[owner])
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    observed: list[list[str]] = []

    def record(argv, **_):
        observed.append(list(argv))
        return subprocess.CompletedProcess(argv, 0, "{}", "")

    monkeypatch.setattr(module.subprocess, "run", record)
    target = tmp_path / "probe target"
    if owner == "inventory":
        module._json_output("cargo", "nextest", "list", target_dir=target)
    elif owner == "clippy":
        module._clippy(tmp_path, target)
    else:
        module._embedded_hash(tmp_path, target)

    assert len(observed) == 1
    argv = observed[0]
    assert argv[:3] == [sys.executable, str(WRAPPER), "--timeout-seconds"], (
        f"{owner} starts Cargo outside AGENTS.md's shared machine lease: {argv}"
    )
    assert float(argv[3]) > 0
    assert argv[4] == "--"
    child = argv[5:]
    assert boundedlease.machine_work(child, config.load(ROOT).locks.bounded)
    assert f"CARGO_TARGET_DIR={target}" in child, "contained env must not replace the probe target"


def test_explicit_probe_environment_survives_the_real_contained_launcher(tmp_path: Path) -> None:
    """No compiler: the actual launcher reports only these fixture-owned keys."""
    selected = {
        "CARGO_TARGET_DIR": str(tmp_path / "probe target"),
        "RUSTC_WRAPPER": "",
        "RUSTC_WORKSPACE_WRAPPER": "",
        "CARGO_BUILD_RUSTC_WRAPPER": "",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": "",
        "CLIPPY_ARGS": "probe arguments",
    }
    child = [
        sys.executable, "-c",
        "import json,os,sys; print(json.dumps({key:os.environ.get(key) for key in sys.argv[1:]}))",
        *selected,
    ]
    result = subprocess.run(
        bounded(child, 30, env=selected), cwd=ROOT,
        capture_output=True, text=True, check=True,
    )
    assert json.loads(result.stdout) == selected


def test_probe_overrides_do_not_hide_cargo_from_the_real_kernel_lease(tmp_path: Path) -> None:
    """Native-free cargo shim: subject is launch/lease routing, never compilation."""
    record = tmp_path / "child-started"
    environment = _environment(tmp_path)
    cargo = tmp_path / "bin/cargo"
    cargo.write_text(f"#!/bin/sh\nprintf started > '{record}'\n")
    cargo.chmod(0o755)
    with (
        _gate_holds_the_machine(tmp_path),
        subprocess.Popen(
            bounded(["cargo", "build"], 30, env={"CARGO_TARGET_DIR": str(tmp_path / "probe")}),
            env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        ) as waiting,
    ):
        try:
            assert waiting.stderr is not None
            assert "waiting:" in waiting.stderr.readline()
            assert not record.exists(), "probe child started while the isolated machine lease was held"
        finally:
            waiting.terminate()
            waiting.communicate(timeout=20)


def test_install_fallback_returns_the_cache_owned_workspace_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem_builder import gatelaunch

    monkeypatch.setenv("CAPSEM_HOME", str(tmp_path / "installed"))
    monkeypatch.setenv("CAPSEM_BIN_SRC", str(tmp_path / "alternate/debug"))
    monkeypatch.setenv("CAPSEM_CACHE_AUTHORITY", str(tmp_path / "authority"))
    monkeypatch.delenv("CAPSEM_INSTALL_SOURCE_CLI", raising=False)
    monkeypatch.delenv("CAPSEM_DEB_INSTALLED", raising=False)
    spec = importlib.util.spec_from_file_location("install_fixture", ROOT / "tests/capsem_install/conftest.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    binary = Path(gatelaunch.contained_environment(ROOT)["CARGO_TARGET_DIR"]) / "debug/capsem"
    calls: list[list[str]] = []

    def built(argv, **_):
        calls.append(list(argv))
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"fixture output")
        return subprocess.CompletedProcess(argv, 0, "", "")

    monkeypatch.setattr(module.subprocess, "run", built)
    assert module.fresh_capsem_binary() == binary
    assert len(calls) == 1
    assert not any("CARGO_TARGET_DIR=" in token for token in calls[0]), (
        "workspace fallback must use the shared owner, never a private probe target"
    )
