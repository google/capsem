"""The network-open host builder has one exact, warm dependency identity."""

from __future__ import annotations

import re
from collections import Counter
from importlib import util
from pathlib import Path
from types import ModuleType
from typing import Any

import yaml
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate import hostimage
from capsem.gate.context import Context
from capsem.gate.packageinputs import pinned_toolchain

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
MUTABLE_TOOLS = (
    "apt",
    "apt-get",
    "aptitude",
    "dnf",
    "yum",
    "apk",
    "brew",
    "uv",
    "pip",
    "pip3",
    "npm",
    "npx",
    "pnpm",
    "cargo",
    "rustup",
    "curl",
    "wget",
)
TOOL_PATTERN = re.compile(
    r"(?<![A-Za-z0-9_.-])(" + "|".join(map(re.escape, MUTABLE_TOOLS)) + r")(?![A-Za-z0-9_.-])"
)


def _tool_inventory(path: Path) -> Counter[str]:
    found: Counter[str] = Counter()
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.lstrip().startswith("#"):
            found.update(set(TOOL_PATTERN.findall(line)))
    return found


def _context(runner: RecordingRunner) -> Context:
    return Context(runner, CONFIG, journal=RecordingJournal())


def _load_cargo_tool_installer() -> ModuleType:
    path = PROJECT_ROOT / "scripts/install-configured-cargo-tools.py"
    spec = util.spec_from_file_location("install_configured_cargo_tools", path)
    assert spec is not None and spec.loader is not None
    module = util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _workflow_steps(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, list):
        return [step for item in value for step in _workflow_steps(item)]
    if isinstance(value, dict):
        found = [value] if "uses" in value or "run" in value else []
        return found + [step for item in value.values() for step in _workflow_steps(item)]
    return []


def test_host_builder_authorities_are_exact_and_config_owned() -> None:
    settings = CONFIG.hostimage

    assert re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", settings.pnpm_version)
    assert pinned_toolchain(PROJECT_ROOT) in settings.rust_image
    assert re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", settings.rust_image)
    assert re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", settings.uv_image)
    assert settings.materialize_network == "default"
    assert set(settings.cargo_tool_args.values()) == {
        "cargo-tauri",
        "cargo-nextest",
        "cargo-llvm-cov",
    }


def test_host_builder_uses_exact_stages_snapshot_and_tool_versions() -> None:
    settings = CONFIG.hostimage
    source = (PROJECT_ROOT / settings.dockerfile).read_text(encoding="utf-8")

    assert "FROM ${RUST_IMAGE} AS rust-toolchain" in source
    assert "FROM ${UV_IMAGE} AS uv-runtime" in source
    assert "astral.sh/uv/install.sh" not in source
    assert "sh.rustup.rs" not in source
    assert 'npm install -g "pnpm@${PNPM_VERSION}"' in source
    assert 'sources-multiarch.sh "$APT_SNAPSHOT_BASE" "$APT_SNAPSHOT_ID"' in source
    assert 'org.capsem.host-builder.input-key="${INPUT_IDENTITY}"' in source
    assert "|| true" not in source
    for argument, tool in settings.cargo_tool_args.items():
        package, version = hostimage.cargo_tool(config=CONFIG, argument=argument)
        assert tool in {crate.name for crate in CONFIG.toolchain.crates}
        assert f"ARG {argument}" in source
        assert f'cargo install {package} --version "${{{argument}}}" --locked' in source
        assert version
    assert "cargo-auditable" not in source


def test_host_builder_mutable_tool_inventory_is_closed() -> None:
    dockerfile = PROJECT_ROOT / CONFIG.hostimage.dockerfile
    assert _tool_inventory(dockerfile) == Counter(
        {
            "cargo": 9,
            "rustup": 4,
            "apt-get": 1,
            "curl": 1,
            "npm": 1,
            "wget": 1,
            "apt": 1,
            "pnpm": 1,
            "uv": 2,
        }
    )


def test_host_builder_multiarch_sources_require_the_shared_snapshot() -> None:
    source = (PROJECT_ROOT / "docker/sources-multiarch.sh").read_text(encoding="utf-8")

    required_base = source.index("${1:?Ubuntu snapshot base is required}")
    required_id = source.index("${2:?Ubuntu snapshot ID is required}")
    architecture = source.index("dpkg --print-architecture")
    assert required_base < architecture and required_id < architecture
    assert "${snapshot_base%/}/${snapshot_id}" in source
    for mutable in ("archive.ubuntu.com", "ports.ubuntu.com", "security.ubuntu.com"):
        assert mutable not in source
    assert "|| true" not in source


def test_host_builder_identity_changes_with_every_declared_input(tmp_path: Path) -> None:
    inputs = (*CONFIG.hostimage.builder_identity_inputs, "config/gate.toml")
    for relative in inputs:
        destination = tmp_path / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes((PROJECT_ROOT / relative).read_bytes())
    rooted = CONFIG.model_copy(update={"root": tmp_path})
    original = hostimage.input_key(rooted)

    first = tmp_path / CONFIG.hostimage.builder_identity_inputs[0]
    first.write_bytes(first.read_bytes() + b"\nchanged\n")

    assert hostimage.input_key(rooted) != original
    changed = rooted.model_copy(
        update={"hostimage": rooted.hostimage.model_copy(update={"pnpm_version": "99.0.0"})}
    )
    assert hostimage.input_key(changed) != hostimage.input_key(rooted)


def test_warm_identity_skips_build_and_cold_identity_builds_exactly_once() -> None:
    identity = hostimage.input_key(CONFIG)
    warm = RecordingRunner(PROJECT_ROOT, replies={"index .Config.Labels": identity})

    hostimage.image(CONFIG).actions[0].perform(_context(warm))

    assert not warm.ran(r"docker build")
    assert len(warm.matching(r"docker run --rm --network none")) == 6
    cold = RecordingRunner(
        PROJECT_ROOT,
        replies={"index .Config.Labels": identity},
        failures=(
            f"docker image inspect --platform {CONFIG.host_arch().docker_platform} "
            f"{CONFIG.hostimage.tag}",
        ),
    )
    hostimage.image(CONFIG).actions[0].perform(_context(cold))
    builds = cold.matching(r"docker build .*Dockerfile\.host-builder")

    assert len(builds) == 1
    assert f"--network {CONFIG.hostimage.materialize_network}" in builds[0]
    assert f"--platform {CONFIG.host_arch().docker_platform}" in builds[0]
    assert f"INPUT_IDENTITY={identity}" in builds[0]
    assert f"PNPM_VERSION={CONFIG.hostimage.pnpm_version}" in builds[0]
    assert f"RUST_IMAGE={CONFIG.hostimage.rust_image}" in builds[0]
    assert f"UV_IMAGE={CONFIG.hostimage.uv_image}" in builds[0]
    assert f"RUST_TOOLCHAIN={pinned_toolchain(PROJECT_ROOT)}" in builds[0]
    assert len(cold.matching(r"docker run --rm --network none")) == 6


def test_macos_release_installs_only_config_owned_cargo_tools() -> None:
    workflow = yaml.safe_load(
        (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    )
    steps = workflow["jobs"]["build-app-macos"]["steps"]
    installer = next(step for step in steps if step.get("name", "").startswith("Install exact"))

    assert installer["run"] == (
        "uv run python scripts/install-configured-cargo-tools.py cargo-tauri cargo-sbom"
    )
    job = str(steps)
    assert "cargo install" not in job
    assert "cargo-auditable" not in job
    source = (PROJECT_ROOT / "scripts/install-configured-cargo-tools.py").read_text(
        encoding="utf-8"
    )
    assert "subprocess.run(tool.install, check=True)" in source
    assert "cargo install" not in source


def test_configured_cargo_tool_installer_executes_the_exact_config_argv(
    monkeypatch,
) -> None:
    module = _load_cargo_tool_installer()
    tool = next(crate for crate in CONFIG.toolchain.crates if crate.name == "cargo-tauri")
    installed = False
    commands: list[tuple[str, ...]] = []

    def probe(argv: tuple[str, ...]) -> str:
        assert argv == tool.probe
        return tool.expected if installed else "cargo-tauri 0.0.0"

    def run(argv: tuple[str, ...], *, check: bool) -> None:
        nonlocal installed
        assert check
        commands.append(argv)
        installed = True

    monkeypatch.setattr(module, "_probe", probe)
    monkeypatch.setattr(module.subprocess, "run", run)

    assert module.main([tool.name]) == 0
    assert commands == [tool.install]


def test_every_workflow_uses_the_exact_config_owned_pnpm_version() -> None:
    workflows = sorted((PROJECT_ROOT / ".github/workflows").glob("*.yaml"))
    setups: list[tuple[Path, dict[str, Any]]] = []
    for path in workflows:
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
        setups.extend(
            (path, step)
            for step in _workflow_steps(document)
            if str(step.get("uses", "")).startswith("pnpm/action-setup@")
        )

    assert setups
    for path, step in setups:
        assert step.get("with", {}).get("version") == CONFIG.hostimage.pnpm_version, path
