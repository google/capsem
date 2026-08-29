"""The network-open host builder has one exact, warm dependency identity."""

from __future__ import annotations

import re
from collections import Counter
from pathlib import Path
from typing import Any

import yaml
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import hostimage
from capsem_builder.gate.context import Context
from capsem_builder.gate.packageinputs import pinned_toolchain
from capsem_builder.image.tools.bootstrap import cargo_tools
from helpers.gate import RecordingJournal, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
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
    # `outside_runner` is the runner a real command holds through its `Egress`
    # resource. The host-image build declares `outside_sandbox=True`, so
    # without one it refuses rather than running inside the sandbox.
    return Context(
        runner,
        CONFIG,
        journal=RecordingJournal(),
        outside_runner=runner,
    )


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
    assert "ARG RUST_TARGETS" in source
    assert 'rustup target add --toolchain "$RUST_TOOLCHAIN" $RUST_TARGETS' in source
    target_install = source.split("rustup target add", maxsplit=1)[1].split(
        "rustup component add", maxsplit=1
    )[0]
    for target in CONFIG.toolchain.rust_targets:
        assert target not in target_install
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

    for action in hostimage.image(CONFIG).actions:
        action.perform(_context(warm))

    assert not warm.ran(r"docker build")
    assert len(warm.matching(r"docker run --rm --network none")) == 6
    cold = RecordingRunner(
        PROJECT_ROOT,
        replies={"index .Config.Labels": identity},
        failures=(f"docker image inspect {CONFIG.hostimage.tag}",),
    )
    materialize, require = hostimage.image(CONFIG).actions
    materialize.perform(_context(cold))
    # The cold inspect is what selects the build.  Once it completes, model
    # the daemon state the following require/probe action must independently
    # verify instead of claiming the image is missing forever.
    cold.fail_on()
    require.perform(_context(cold))
    builds = cold.matching(r"docker build .*Dockerfile\.host-builder")

    assert len(builds) == 1
    assert f"--network {CONFIG.hostimage.materialize_network}" in builds[0]
    assert f"--platform {CONFIG.host_arch().docker_platform}" in builds[0]
    assert f"INPUT_IDENTITY={identity}" in builds[0]
    assert f"PNPM_VERSION={CONFIG.hostimage.pnpm_version}" in builds[0]
    assert f"RUST_IMAGE={CONFIG.hostimage.rust_image}" in builds[0]
    assert f"UV_IMAGE={CONFIG.hostimage.uv_image}" in builds[0]
    assert f"RUST_TOOLCHAIN={pinned_toolchain(PROJECT_ROOT)}" in builds[0]
    assert "RUST_TARGETS=" + " ".join(CONFIG.toolchain.rust_targets) in builds[0]
    assert len(cold.matching(r"docker run --rm --network none")) == 6


def test_macos_release_installs_only_config_owned_cargo_tools() -> None:
    workflow = yaml.safe_load(
        (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    )
    steps = workflow["jobs"]["build-app-macos"]["steps"]
    installer = next(step for step in steps if step.get("name", "").startswith("Install exact"))

    assert installer["run"] == (
        "uv run --project build_system --frozen python scripts/install-configured-cargo-tools.py cargo-tauri cargo-sbom"
    )
    job = str(steps)
    assert "cargo install" not in job
    assert "cargo-auditable" not in job
    source = (
        PROJECT_ROOT
        / "build_system/builder/image/tools/bootstrap/cargo_tools.py"
    ).read_text(
        encoding="utf-8"
    )
    assert "subprocess.run(tool.install, check=True)" in source
    assert "cargo install" not in source


def test_configured_cargo_tool_installer_executes_the_exact_config_argv(
    monkeypatch,
) -> None:
    module = cargo_tools
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


def test_every_complete_rust_toolchain_setup_uses_the_config_owned_targets() -> None:
    expected = set(CONFIG.toolchain.rust_targets)
    setups: list[tuple[Path, str]] = []
    for path in sorted((PROJECT_ROOT / ".github/workflows").glob("*.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
        for step in _workflow_steps(document):
            if not str(step.get("uses", "")).startswith("dtolnay/rust-toolchain@"):
                continue
            targets = str(step.get("with", {}).get("targets", ""))
            selected = {target.strip() for target in targets.split(",") if target.strip()}
            if {
                "aarch64-unknown-linux-musl",
                "x86_64-unknown-linux-musl",
            } <= selected:
                setups.append((path, targets))

    assert setups
    for path, targets in setups:
        assert {target.strip() for target in targets.split(",")} == expected, path


def test_probing_a_tool_that_is_not_installed_yet_is_not_fatal() -> None:
    """A fresh runner has none of these, which is the whole reason to install.

    `check=False` covers a tool that runs and exits non-zero. It does nothing
    for one that is not on PATH: `subprocess.run` raises before it can run
    anything. The macOS release job died exactly here -- FileNotFoundError,
    'cargo-tauri' -- having never reached the install it was about to do.
    """
    module = cargo_tools

    assert module._probe(("capsem-tool-that-is-not-installed", "--version")) == ""


def test_a_tool_that_is_absent_is_installed_rather_than_fatal(monkeypatch) -> None:
    """Absent and wrong-version mean the same thing to the caller: install it."""
    module = cargo_tools
    tool = next(crate for crate in CONFIG.toolchain.crates if crate.name == "cargo-tauri")
    installs: list[tuple[str, ...]] = []
    present = False

    def fake_run(argv, **kwargs):
        nonlocal present
        if tuple(argv) == tuple(tool.install):
            installs.append(tuple(argv))
            present = True
            return module.subprocess.CompletedProcess(argv, 0, "", "")
        if not present:
            raise FileNotFoundError(2, "No such file or directory", argv[0])
        return module.subprocess.CompletedProcess(argv, 0, tool.expected, "")

    monkeypatch.setattr(module.subprocess, "run", fake_run)

    assert module.main([tool.name]) == 0
    assert installs == [tuple(tool.install)]
