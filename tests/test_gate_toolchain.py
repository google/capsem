from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate import toolchain
from capsem.gate.context import Context
from capsem.gate.errors import GateError
from capsem.gate.packageinputs import pinned_toolchain

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
PIN = pinned_toolchain(PROJECT_ROOT)


EXPECTED_TOOLS = {
    "cargo-nextest": (("cargo-nextest", "--version"), "cargo-nextest 0.9.137", "0.9.137"),
    "cargo-llvm-cov": (
        ("cargo-llvm-cov", "llvm-cov", "--version"),
        "cargo-llvm-cov 0.8.5",
        "0.8.5",
    ),
    "b3sum": (("b3sum", "--version"), "b3sum 1.8.5", "1.8.5"),
    "cargo-audit": (("cargo-audit", "--version"), "cargo-audit 0.22.1", "0.22.1"),
    "cargo-sbom": (("cargo-sbom", "--version"), "cargo-sbom 0.10.0", "0.10.0"),
    "cargo-tauri": (("cargo-tauri", "--version"), "tauri-cli 2.11.1", "2.11.1"),
}


def _rust_replies(*, targets: str, components: str) -> dict[str, str]:
    replies = {"target list": targets, "component list": components}
    replies.update(
        {
            " ".join(probe): expected
            for probe, expected, _version in EXPECTED_TOOLS.values()
        }
    )
    return replies


def test_every_config_owned_cargo_tool_has_one_exact_version_probe() -> None:
    configured = {crate.name: crate for crate in CONFIG.toolchain.crates}

    assert configured.keys() == EXPECTED_TOOLS.keys()
    for name, (probe, expected, version) in EXPECTED_TOOLS.items():
        crate = configured[name]
        package = "tauri-cli" if name == "cargo-tauri" else name
        assert crate.probe == probe
        assert crate.expected == expected
        assert crate.install == (
            "cargo",
            "install",
            package,
            "--version",
            version,
            "--locked",
        )


def test_an_outdated_cargo_tool_is_reinstalled_and_must_verify(monkeypatch) -> None:
    replies = _rust_replies(
        targets="aarch64-unknown-linux-musl\nx86_64-unknown-linux-musl\n",
        components="llvm-tools-x86_64-unknown-linux-gnu\n",
    )
    replies["cargo-nextest --version"] = "cargo-nextest 0.9.136"
    runner = RecordingRunner(PROJECT_ROOT, replies=replies)
    monkeypatch.setattr(toolchain.shutil, "which", lambda _name: "/tool")

    with pytest.raises(GateError, match=r"cargo-nextest did not provide cargo-nextest 0\.9\.137"):
        toolchain.rust(CONFIG).run(Context(runner, CONFIG))

    assert "cargo install cargo-nextest --version 0.9.137 --locked" in runner.rendered


def test_sealed_fast_ci_preinstalls_the_same_nextest_pin() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text()
    tools = next(line for line in workflow.splitlines() if "tool: cargo-audit@" in line)

    assert "cargo-nextest@0.9.137" in tools


def test_rust_setup_names_the_checked_in_toolchain_for_every_rustup_probe(
    monkeypatch,
) -> None:
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies=_rust_replies(
            targets="aarch64-unknown-linux-musl\nx86_64-unknown-linux-musl\n",
            components="llvm-tools-x86_64-unknown-linux-gnu\n",
        ),
    )
    monkeypatch.setattr(toolchain.shutil, "which", lambda _name: "/tool")

    toolchain.rust(CONFIG).run(Context(runner, CONFIG))

    issued = "\n".join(runner.rendered)
    assert f"rustup target list --toolchain {PIN} --installed" in issued
    assert f"rustup component list --toolchain {PIN} --installed" in issued


def test_missing_targets_are_installed_into_the_checked_in_toolchain(monkeypatch) -> None:
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies=_rust_replies(targets="", components=""),
    )
    monkeypatch.setattr(toolchain.shutil, "which", lambda _name: "/tool")

    toolchain.rust(CONFIG).run(Context(runner, CONFIG))

    issued = "\n".join(runner.rendered)
    assert f"rustup target add --toolchain {PIN} aarch64-unknown-linux-musl" in issued
    assert f"rustup target add --toolchain {PIN} x86_64-unknown-linux-musl" in issued
    assert f"rustup component add --toolchain {PIN} llvm-tools" in issued
