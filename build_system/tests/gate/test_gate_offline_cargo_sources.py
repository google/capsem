"""A gate that drops the network must already hold every crate `Cargo.lock` names.

`bootstrap.sh` fetched the workspace crates once, at setup, and nothing checked
again. When `origin/main` added `utoipa` to `Cargo.lock`, a `focus-test
kingslanding` ran for minutes, then failed at `assets.recovery-dependencies` --
`cargo run -p capsem-admin` inside the no-network sandbox -- with a wall of
`spurious network error ... Could not resolve host: index.crates.io`. Nothing
in it said the fix was a `cargo fetch --locked` outside the sandbox.

`cargo fetch --locked --offline` answers the question without the network: it
exits non-zero when a locked crate is not in the local registry. The gate asks
it at the last moment it still has a network (just before it wraps itself in
the sandbox) and `just doctor` asks it too, with `just doctor fix` as the
answer to both.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import sandbox
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.harnessschema import SandboxConfig
from helpers.gate import RecordingRunner
from pydantic import ValidationError

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
PROBE = " ".join(CONFIG.sandbox.cargo_offline_probe)
DOCTOR = PROJECT_ROOT / "build_system/scripts/doctor"


def _enter(monkeypatch: pytest.MonkeyPatch, runner: RecordingRunner, *, active: bool = False,
           default: sandbox.SandboxMode = sandbox.ENFORCE) -> tuple[str, ...]:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: active)
    return sandbox.applied(
        CONFIG, runner, default=default, requested=None, argv=("python3", "-c", "pass")
    )


def test_the_probe_is_offline_and_locked() -> None:
    assert CONFIG.sandbox.cargo_offline_probe == ("cargo", "fetch", "--locked", "--offline")


@pytest.mark.parametrize("dropped", ["--offline", "--locked"])
def test_the_probe_cannot_be_configured_into_a_fetch_or_a_resolve(dropped: str) -> None:
    """Without --offline the probe would fetch, and so hide the very state it
    reports; without --locked it could re-resolve and pass on a stale lock."""
    raw = CONFIG.sandbox.model_dump()
    raw["cargo_offline_probe"] = tuple(
        part for part in CONFIG.sandbox.cargo_offline_probe if part != dropped
    )

    with pytest.raises(ValidationError):
        SandboxConfig.model_validate(raw)


def test_entering_the_sandbox_probes_the_local_registry_first(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runner = RecordingRunner(PROJECT_ROOT)

    wrapped = _enter(monkeypatch, runner)

    assert wrapped[0] == CONFIG.sandbox.linux_command
    probes = [command for command in runner.commands if command.argv == tuple(
        CONFIG.sandbox.cargo_offline_probe
    )]
    assert len(probes) == 1
    assert probes[0].cwd == CONFIG.root


def test_a_stale_registry_is_refused_before_the_sandbox_with_the_fix(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runner = RecordingRunner(PROJECT_ROOT, failures=[PROBE])

    with pytest.raises(GateError) as refused:
        _enter(monkeypatch, runner)

    message = str(refused.value)
    assert "Cargo.lock" in message
    assert "`just doctor fix`" in message
    assert "no network" in message


@pytest.mark.parametrize(
    ("active", "default"), [(True, sandbox.ENFORCE), (False, sandbox.OFF)]
)
def test_no_probe_when_the_network_is_not_being_dropped_here(
    monkeypatch: pytest.MonkeyPatch, active: bool, default: sandbox.SandboxMode
) -> None:
    """Already inside, the probe was answered by the process that entered;
    with the sandbox off, cargo may still fetch what it needs."""
    runner = RecordingRunner(PROJECT_ROOT, failures=[PROBE])

    _enter(monkeypatch, runner, active=active, default=default)

    assert not runner.ran(PROBE)


def test_the_probe_really_fails_on_an_empty_registry_without_the_network(
    tmp_path: Path,
) -> None:
    """The premise, measured: an empty CARGO_HOME cannot satisfy the lock, and
    cargo says so offline instead of reaching for the index."""
    completed = subprocess.run(
        CONFIG.sandbox.cargo_offline_probe,
        cwd=PROJECT_ROOT,
        env={**os.environ, "CARGO_HOME": str(tmp_path)},
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )

    assert completed.returncode != 0
    assert "offline" in completed.stderr
    assert not (tmp_path / "registry" / "cache").exists()


def test_doctor_checks_the_same_probe_and_fixes_it_with_a_locked_fetch() -> None:
    run = (DOCTOR / "doctor-run.sh").read_text(encoding="utf-8")
    common = (DOCTOR / "doctor-common.sh").read_text(encoding="utf-8")

    assert PROBE in run
    assert "fixable cargo-fetch" in run
    assert "cargo fetch --locked\n" in common
    order = [common.index(f"_reg {name} ") for name in (
        "llvm-tools", "cargo-fetch", "gate-cargo-tools", "build-assets", "pack-initrd"
    )]
    assert order == sorted(order), "crates must be local before anything builds with them"


def test_doctor_names_the_recipe_that_exists() -> None:
    """It told operators to run `just doctor-fix`, which is not a recipe."""
    run = (DOCTOR / "doctor-run.sh").read_text(encoding="utf-8")

    assert "doctor-fix" not in run
    assert "just doctor fix" in run


def test_doctor_fix_refuses_to_fetch_from_inside_a_gate() -> None:
    """Inside the sandbox the fetch would be the same DNS wall; say what to do."""
    completed = subprocess.run(
        [
            "bash",
            "-c",
            'eval "$(sed -n "/^_doctor_fetch_cargo_sources()/,/^}/p" "$1")"; '
            "cargo() { echo FETCHED; }; _doctor_fetch_cargo_sources",
            "doctor",
            str(DOCTOR / "doctor-common.sh"),
        ],
        cwd=PROJECT_ROOT,
        env={**os.environ, "CAPSEM_GATE_RUN": "capsem-gate focus-test"},
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )

    assert completed.returncode != 0
    assert "FETCHED" not in completed.stdout
    assert "just doctor fix" in completed.stderr


def test_bootstrap_fetches_through_the_doctor_fix_rather_than_its_own_copy() -> None:
    """One way: bootstrap's phase 3 runs `doctor-common.sh --fix`, which owns it."""
    bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text(encoding="utf-8")

    assert "cargo fetch --locked" not in bootstrap
    assert "doctor-common.sh\" --fix" in bootstrap
