from pathlib import Path

from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate import toolchain
from capsem.gate.context import Context
from capsem.gate.packageinputs import pinned_toolchain

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
PIN = pinned_toolchain(PROJECT_ROOT)


def test_rust_setup_names_the_checked_in_toolchain_for_every_rustup_probe(
    monkeypatch,
) -> None:
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies={
            "target list": "aarch64-unknown-linux-musl\nx86_64-unknown-linux-musl\n",
            "component list": "llvm-tools-x86_64-unknown-linux-gnu\n",
        },
    )
    monkeypatch.setattr(toolchain.shutil, "which", lambda _name: "/tool")

    toolchain.rust(CONFIG).run(Context(runner, CONFIG))

    issued = "\n".join(runner.rendered)
    assert f"rustup target list --toolchain {PIN} --installed" in issued
    assert f"rustup component list --toolchain {PIN} --installed" in issued


def test_missing_targets_are_installed_into_the_checked_in_toolchain(monkeypatch) -> None:
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies={"target list": "", "component list": ""},
    )
    monkeypatch.setattr(toolchain.shutil, "which", lambda _name: "/tool")

    toolchain.rust(CONFIG).run(Context(runner, CONFIG))

    issued = "\n".join(runner.rendered)
    assert f"rustup target add --toolchain {PIN} aarch64-unknown-linux-musl" in issued
    assert f"rustup target add --toolchain {PIN} x86_64-unknown-linux-musl" in issued
    assert f"rustup component add --toolchain {PIN} llvm-tools" in issued
