"""Asset builds have one visible materialization edge and a sealed tail.

This is intentionally an executable inventory rather than an audit checklist.
Adding a package manager, installer, fetcher, or package-runner anywhere in the
publishable asset rail changes this test.  The author must then either place it
inside the declared materializer or remove it from the sealed phase.
"""

from __future__ import annotations

import ast
import inspect
import re
from collections import Counter
from pathlib import Path

import pytest
import yaml
from helpers.gate import RecordingRunner
from helpers.workflow_contract import canonical_shell_commands

from capsem.builder import assettools
from capsem.builder import docker as asset_docker
from capsem.builder.config import load_guest_config
from capsem.gate import config as gate_config
from capsem.gate import imagebases
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
BUILD = load_guest_config(PROJECT_ROOT / "config/docker/image").build

# Package managers, language installers/runners, and direct download clients.
# Keep this broad: a new ecosystem must be classified before release, not after
# it surprises a late two-hour build.
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


def _function_string_inventory(path: Path) -> dict[str, Counter[str]]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    found: dict[str, Counter[str]] = {}
    for node in ast.walk(tree):
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        strings = Counter()
        for child in ast.walk(node):
            if isinstance(child, ast.Constant) and isinstance(child.value, str):
                strings.update(TOOL_PATTERN.findall(child.value))
        if strings:
            found[node.name] = strings
    return found


def _source_line_inventory(path: Path) -> Counter[str]:
    found = Counter()
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        found.update(set(TOOL_PATTERN.findall(line)))
    return found


def test_mutable_tool_vocabulary_covers_every_supported_ecosystem() -> None:
    assert set(MUTABLE_TOOLS) >= {
        "apt",
        "apt-get",
        "dnf",
        "yum",
        "apk",
        "brew",
        "uv",
        "pip",
        "npm",
        "npx",
        "pnpm",
        "cargo",
        "rustup",
        "curl",
        "wget",
    }


def test_asset_python_has_no_unclassified_mutable_tool_commands() -> None:
    inventory = _function_string_inventory(PROJECT_ROOT / "src/capsem/builder/docker.py")

    # `_rootfs_context` declares commands inserted into the network-open rootfs
    # Dockerfile. `container_compile_agent` is the one sealed compiler client.
    # No post-build packer, probe, inventory, or OBOM helper may grow its own
    # package-manager/fetch path.
    assert set(inventory) == {
        "_rootfs_context",  # declarations rendered into the materializer
        "build_version_script",  # read-only version commands
        "container_compile_agent",  # separately proved locked/offline below
        "extract_software_inventory",  # dpkg-query, pip list, npm ls
        "sync_container_clock",  # pre-materialization Colima clock repair
    }
    # These are read-only inventory commands. Ratchet their executable tool
    # vocabulary directly instead of scanning prose: a docstring such as
    # "installed packages" must not look like an install command.
    assert inventory["extract_software_inventory"] == Counter({"npm": 3, "pip": 1})


def test_container_guest_compile_is_locked_offline_and_network_denied() -> None:
    source = inspect.getsource(asset_docker.container_compile_agent)
    assert '"--network"' in source
    assert "runtime_network" in source
    assert "cargo build --locked --offline" in source


def test_all_container_probes_are_network_denied() -> None:
    source = inspect.getsource(asset_docker._container_output)
    assert '"--network"' in source
    assert '"none"' in source


def test_every_guest_compile_uses_the_same_sealed_helper() -> None:
    source = inspect.getsource(asset_docker.cross_compile_agent)
    assert "return container_compile_agent(" in source
    assert "run_cmd" not in source
    assert "rustup" not in source
    assert "cargo" not in source


def test_asset_docker_build_network_is_phase_owned_by_config() -> None:
    config = load_guest_config(PROJECT_ROOT / "config/docker/image")
    assert config.build.materialize_network == "default"
    assert config.build.guest_rust_builder.runtime_network == "none"
    assert config.build.asset_dependencies.source_build_network == "none"

    materialize = inspect.getsource(asset_docker.materialize_asset_dependencies)
    source = inspect.getsource(asset_docker.build_image)
    assert "network=config.build.materialize_network" in materialize
    assert "network=config.build.asset_dependencies.source_build_network" in source
    assert "ci_cache=False" in source


def test_asset_tool_smokes_erofs_through_its_portable_help_contract() -> None:
    source = (PROJECT_ROOT / BUILD.asset_tools.dockerfile).read_text(encoding="utf-8")

    assert "mkfs.erofs --help > /dev/null" in source
    assert "mkfs.erofs -V" not in source


@pytest.mark.parametrize(
    "relative",
    (
        BUILD.asset_tools.dockerfile,
        BUILD.guest_rust_builder.dockerfile,
        f"config/docker/{BUILD.asset_dependencies.kernel_template}",
        f"config/docker/{BUILD.asset_dependencies.rootfs_template}",
    ),
)
def test_required_helper_base_waivers_never_supply_a_fallback(relative: str) -> None:
    lines = (PROJECT_ROOT / relative).read_text(encoding="utf-8").splitlines()

    assert lines[0] == "# check=skip=InvalidDefaultArgInFrom"
    assert "ARG BASE" in lines
    assert "ARG BASE=" not in "\n".join(lines)
    assert "FROM ${BASE}" in lines


def test_declared_asset_materializers_match_the_exact_mutator_inventory() -> None:
    expected = {
        "config/docker/Dockerfile.kernel-dependencies.j2": Counter(
            {"apt": 5, "apt-get": 2, "wget": 2}
        ),
        "config/docker/Dockerfile.rootfs-dependencies.j2": Counter(
            {
                "npm": 5,
                "apt": 4,
                "apt-get": 3,
                "curl": 3,
                "uv": 3,
                "pip": 2,
                "pip3": 1,
                "npx": 1,
            }
        ),
        "config/profiles/co-work/build.sh": Counter({"curl": 3}),
        "config/profiles/code/build.sh": Counter({"curl": 3}),
        "docker/Dockerfile.asset-tools": Counter({"apt": 5, "curl": 3, "apt-get": 2}),
        "docker/Dockerfile.guest-rust-builder": Counter({"rustup": 3, "apk": 1, "cargo": 1}),
    }
    candidates = {
        *PROJECT_ROOT.glob("config/docker/**/*.j2"),
        *PROJECT_ROOT.glob("config/profiles/*/build.sh"),
        PROJECT_ROOT / "docker/Dockerfile.asset-tools",
        PROJECT_ROOT / "docker/Dockerfile.guest-rust-builder",
    }
    actual = {
        path.relative_to(PROJECT_ROOT).as_posix(): inventory
        for path in sorted(candidates)
        if (inventory := _source_line_inventory(path))
    }
    assert actual == expected


def test_guest_cross_compiler_packages_are_exact_and_input_keyed() -> None:
    settings = BUILD.guest_rust_builder
    assert settings.cross_packages == ("clang21=21.1.2-r2",)
    assert all("=" in package for package in settings.cross_packages)

    source = (PROJECT_ROOT / settings.dockerfile).read_text(encoding="utf-8")
    assert "apk add --no-cache ${CROSS_PACKAGES}" in source
    assert "apk add --no-cache clang" not in source


def test_asset_materializer_builds_name_their_network_explicitly() -> None:
    source = inspect.getsource(asset_docker.materialize_asset_dependencies)
    assert "network=config.build.materialize_network" in source


def test_profile_release_asset_job_has_no_parallel_package_authority() -> None:
    workflow = yaml.safe_load(
        (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(encoding="utf-8")
    )
    steps = workflow["jobs"]["build-assets"]["steps"]
    commands = tuple(
        command
        for item in steps
        if isinstance(item, dict) and isinstance(item.get("run"), str)
        for command in canonical_shell_commands(item["run"])
    )
    tokens = tuple(token for command in commands for token in command)

    # The fresh runner materializes only the locked Python environment needed
    # to invoke the gate. Guest Rust, EROFS, cdxgen, and validation all belong
    # to the gate's config-owned helpers; workflow-local apt/npm/cargo/brew
    # commands would be a second authority and must change this exact counter.
    assert Counter(token for token in tokens if token in MUTABLE_TOOLS) == Counter({"uv": 1})
    assert ("uv", "sync", "--frozen") in commands
    assert "CAPSEM_CDXGEN_CMD" not in tokens
    assert "musl-gcc" not in tokens


def test_asset_tool_identity_changes_with_dockerfile_and_config(tmp_path: Path) -> None:
    dockerfile = tmp_path / BUILD.asset_tools.dockerfile
    dockerfile.parent.mkdir(parents=True)
    dockerfile.write_bytes((PROJECT_ROOT / BUILD.asset_tools.dockerfile).read_bytes())
    original = assettools.image_tag(BUILD, "x86_64", tmp_path)

    dockerfile.write_text("changed helper\n", encoding="utf-8")
    assert assettools.image_tag(BUILD, "x86_64", tmp_path) != original

    settings = BUILD.asset_tools
    changed_settings = settings.model_copy(update={"debian_snapshot_id": "20260809T000000Z"})
    changed_build = BUILD.model_copy(update={"asset_tools": changed_settings})
    assert assettools.image_tag(changed_build, "x86_64", PROJECT_ROOT) != assettools.image_tag(
        BUILD, "x86_64", PROJECT_ROOT
    )


def test_asset_tool_helper_replaces_inherited_apt_authority_before_fetch() -> None:
    source = (PROJECT_ROOT / BUILD.asset_tools.dockerfile).read_text(encoding="utf-8")

    remove = source.index("rm -f /etc/apt/sources.list.d/*")
    write = source.index("> /etc/apt/sources.list")
    update = source.index("apt-get -o Acquire::Check-Date=false update")
    assert remove < write < update
    assert "deb.debian.org" not in source
    assert "security.debian.org" not in source


def test_asset_tool_materializer_is_the_only_network_open_helper_build() -> None:
    config = gate_config.load(PROJECT_ROOT)
    name = config.host_arch().name
    arch = BUILD.architectures[name]
    tag = assettools.image_tag(BUILD, name, PROJECT_ROOT)
    runner = RecordingRunner(
        PROJECT_ROOT,
        failures=(f"docker image inspect {tag}",),
    )

    imagebases.materialize_asset_tools(runner, config)

    builds = runner.matching(r"docker build .*Dockerfile\.asset-tools")
    assert len(builds) == 1
    assert f"--platform {arch.docker_platform}" in builds[0]
    assert "--network default" in builds[0]
    assert f"BASE={arch.base_image}" in builds[0]
    assert f"INPUT_IDENTITY={tag}" in builds[0]
    assert runner.ran(r"index \.Config\.Labels.*org\.capsem\.asset-tools\.input-key")
    assert any("sealed asset tools identity" in note for note in runner.notes)


def test_asset_tool_materializer_refuses_a_poisoned_warm_tag() -> None:
    config = gate_config.load(PROJECT_ROOT)
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies={"index .Config.Labels": "forged-input-key"},
    )

    with pytest.raises(GateError, match="poisoned warm tag"):
        imagebases.materialize_asset_tools(runner, config)

    assert not runner.ran(r"docker build")
