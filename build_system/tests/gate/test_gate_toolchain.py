from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import toolchain
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.packageinputs import pinned_toolchain
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
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
        {" ".join(probe): expected for probe, expected, _version in EXPECTED_TOOLS.values()}
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
    import re
    import tomllib

    workflow = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text()
    chosen = re.search(r"--sets ([a-z,]+)", workflow)
    assert chosen is not None, "the fast gate no longer selects a declared tool set"

    sets = tomllib.loads(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )["toolchain"]["sets"]
    members: set[str] = set()
    for label in chosen.group(1).split(","):
        members.update(sets[label])

    # The pin itself lives in `[[toolchain.crates]]`; what this holds is that
    # the sealed fast lane installs the same nextest the rest of the gate does.
    assert "cargo-nextest" in members


def test_ort_distributions_are_one_toolchain_authority_for_linux_and_macos() -> None:
    configured = CONFIG.toolchain.ort.distributions

    assert {arch.rust_target for arch in CONFIG.architectures.values()} <= configured.keys()
    assert "aarch64-apple-darwin" in configured
    for distribution in configured.values():
        assert distribution.url.startswith("https://cdn.pyke.io/")
        assert len(distribution.sha256) == 64


def test_ort_materializer_selects_apple_silicon_distribution(monkeypatch) -> None:
    monkeypatch.setattr(toolchain.host, "system", lambda: "Darwin")
    monkeypatch.setattr(toolchain.host, "machine", lambda: "arm64")

    selected = CONFIG.toolchain.ort.distributions["aarch64-apple-darwin"]
    rendered = "\n".join(toolchain.ort(CONFIG, toolchain.OrtConsumer.FAST).render())
    environment = toolchain.ort_environment(CONFIG, toolchain.OrtConsumer.FAST)

    assert selected.url in rendered
    assert selected.sha256 in rendered
    assert "fast-aarch64-apple-darwin" in rendered
    assert environment == {
        CONFIG.toolchain.ort.strategy_variable: CONFIG.toolchain.ort.strategy,
        CONFIG.toolchain.ort.location_variable: str(
            PROJECT_ROOT
            / CONFIG.toolchain.ort.output_template.format(
                consumer="fast",
                target="aarch64-apple-darwin",
                sha256=selected.sha256,
            )
        ),
    }


def test_ort_materializer_refuses_an_unconfigured_host(monkeypatch) -> None:
    monkeypatch.setattr(toolchain.host, "system", lambda: "Darwin")
    monkeypatch.setattr(toolchain.host, "machine", lambda: "x86_64")

    with pytest.raises(GateError, match="no distribution for Darwin/x86_64"):
        toolchain.ort(CONFIG, toolchain.OrtConsumer.FAST)


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


def test_missing_rust_items_materialize_through_the_narrow_egress(monkeypatch) -> None:
    ordinary = RecordingRunner(
        PROJECT_ROOT,
        replies=_rust_replies(targets="", components=""),
    )
    capability = RecordingRunner(PROJECT_ROOT)
    monkeypatch.setattr(toolchain.shutil, "which", lambda _name: None)

    toolchain.rust(CONFIG).run(Context(ordinary, CONFIG, outside_runner=capability))

    ordinary_commands = "\n".join(ordinary.rendered)
    materializers = "\n".join(capability.rendered)
    assert "rustup target list" in ordinary_commands
    assert "rustup component list" in ordinary_commands
    assert "rustup target add" not in ordinary_commands
    assert "rustup component add" not in ordinary_commands
    assert "cargo install" not in ordinary_commands
    for target in CONFIG.toolchain.rust_targets:
        assert f"rustup target add --toolchain {PIN} {target}" in materializers
    for component in CONFIG.toolchain.rust_components:
        assert f"rustup component add --toolchain {PIN} {component}" in materializers
    for crate in CONFIG.toolchain.crates:
        assert " ".join(crate.install) in materializers
