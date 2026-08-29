"""Asset builds have one visible materialization edge and a sealed tail.

This is intentionally an executable inventory rather than an audit checklist.
Adding a package manager, installer, fetcher, or package-runner anywhere in the
publishable asset rail changes this test.  The author must then either place it
inside the declared materializer or remove it from the sealed phase.
"""

from __future__ import annotations

import ast
import inspect
import json
import os
import re
import shlex
import stat
import subprocess
import sys
from collections import Counter
from pathlib import Path

import pytest
import yaml
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import imagebases
from capsem_builder.gate.errors import GateError
from capsem_builder.image import assettools
from capsem_builder.image import docker as asset_docker
from capsem_builder.image.config import load_guest_config
from helpers.gate import RecordingRunner
from helpers.workflow_contract import canonical_shell_commands

SEALED_MACOS = (
    sys.platform == "darwin" and os.environ.get("CAPSEM_GATE_COMMAND_SANDBOX_MODE") == "enforce"
)

PROJECT_ROOT = Path(__file__).resolve().parents[3]
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


def _dockerfile_instructions(source: str) -> tuple[str, ...]:
    """Return whitespace-normalized Dockerfile instructions.

    The contract intentionally ignores comments, indentation, and line wrapping
    so harmless Dockerfile refactors do not look like security regressions.
    """
    instructions: list[str] = []
    current: list[str] = []
    for raw_line in source.splitlines():
        line = raw_line.strip()
        if current:
            current.append(line)
        elif re.match(r"^[A-Z][A-Z0-9]*\s", line):
            current = [line]
        else:
            continue

        if line.endswith("\\"):
            continue
        instructions.append(" ".join(part.removesuffix("\\").rstrip() for part in current))
        current = []

    assert not current, "Dockerfile ends inside a continued instruction"
    return tuple(instructions)


def _rootfs_privilege_hardening(source: str) -> str:
    """Validate and return the one fail-closed privilege hardening command."""
    instructions = _dockerfile_instructions(source)
    matches = [
        (index, instruction)
        for index, instruction in enumerate(instructions)
        if instruction.startswith("RUN ") and "chmod u-s,g-s" in instruction
    ]
    assert len(matches) == 1, "rootfs must have exactly one privilege-bit scrub"
    hardening_index, hardening = matches[0]

    assert "set -eu" in hardening, "privilege-bit scrub must fail on every error"
    assert "||" not in hardening, "privilege-bit scrub must not swallow failures"
    assert "/dev/null" not in hardening, "privilege-bit failures must remain visible"
    assert hardening.count("find / -xdev") == 2, (
        "rootfs must strip privileged files and independently verify none remain"
    )
    assert 'test -z "$remaining"' in hardening, (
        "rootfs must fail when privileged files remain after the scrub"
    )
    strip = hardening.index("chmod u-s,g-s")
    verify_find = hardening.index("find / -xdev", strip)
    verify_empty = hardening.index('test -z "$remaining"')
    assert strip < verify_find < verify_empty

    later = instructions[hardening_index + 1 :]
    assert not any(instruction.startswith(("COPY ", "ADD ")) for instruction in later), (
        "rootfs content must not be copied after privilege verification"
    )
    forbidden_later = re.compile(
        r"(?<![A-Za-z0-9_.-])(?:apt|apt-get|aptitude|dnf|yum|apk|brew|uv|pip|pip3|"
        r"npm|npx|pnpm|cargo|rustup|curl|wget|install)(?![A-Za-z0-9_.-])"
    )
    assert not any(forbidden_later.search(instruction) for instruction in later), (
        "rootfs dependency acquisition must finish before privilege verification"
    )
    return hardening


def test_rootfs_privilege_hardening_is_fail_closed() -> None:
    source = (PROJECT_ROOT / "config/docker/Dockerfile.rootfs.j2").read_text(encoding="utf-8")

    _rootfs_privilege_hardening(source)


def test_rootfs_privilege_hardening_accepts_equivalent_formatting() -> None:
    source = (PROJECT_ROOT / "config/docker/Dockerfile.rootfs.j2").read_text(encoding="utf-8")
    reformatted = source.replace(
        "# Every dependency is already present, so this final hardening is the last mutation.",
        "# Equivalent prose and whitespace must not weaken this executable contract.",
    ).replace("RUN set -eu;", "RUN    set -eu;")

    _rootfs_privilege_hardening(reformatted)


@pytest.mark.parametrize(
    ("needle", "replacement"),
    (
        (
            "-exec chmod u-s,g-s {} +;",
            "-exec chmod u-s,g-s {} + || true;",
        ),
        (
            "-exec chmod u-s,g-s {} +;",
            "-exec chmod u-s,g-s {} + 2>/dev/null;",
        ),
        ('test -z "$remaining"', "true"),
    ),
    ids=("swallowed-exit", "hidden-errors", "deleted-verification"),
)
def test_rootfs_privilege_hardening_rejects_fail_open_mutations(
    needle: str, replacement: str
) -> None:
    source = (PROJECT_ROOT / "config/docker/Dockerfile.rootfs.j2").read_text(encoding="utf-8")
    assert needle in source

    with pytest.raises(AssertionError):
        _rootfs_privilege_hardening(source.replace(needle, replacement, 1))


def test_rootfs_privilege_hardening_rejects_later_acquisition() -> None:
    source = (PROJECT_ROOT / "config/docker/Dockerfile.rootfs.j2").read_text(encoding="utf-8")

    with pytest.raises(AssertionError, match="acquisition must finish"):
        _rootfs_privilege_hardening(source + "\nRUN npm install late-package\n")


@pytest.mark.skipif(
    SEALED_MACOS,
    reason="Seatbelt strips synthetic Linux privilege bits; Linux CI executes this proof",
)
def test_rootfs_privilege_hardening_strips_a_synthetic_file(tmp_path: Path) -> None:
    source = (PROJECT_ROOT / "config/docker/Dockerfile.rootfs.j2").read_text(encoding="utf-8")
    instruction = _rootfs_privilege_hardening(source)
    root = tmp_path / "rootfs"
    root.mkdir()
    privileged = root / "synthetic-privileged"
    privileged.write_text("fixture\n", encoding="utf-8")
    privileged.chmod(0o6755)
    privilege_bits = stat.S_ISUID | stat.S_ISGID
    assert privileged.stat().st_mode & privilege_bits == privilege_bits

    command = instruction.removeprefix("RUN ").replace(
        "find / -xdev", f"find {shlex.quote(str(root))} -xdev"
    )
    subprocess.run(["sh", "-c", command], check=True)

    assert privileged.stat().st_mode & privilege_bits == 0


def test_guest_privilege_scans_propagate_find_failures() -> None:
    source = (PROJECT_ROOT / "guest/artifacts/diagnostics/test_sandbox.py").read_text(
        encoding="utf-8"
    )

    for privilege in ("setuid", "setgid"):
        function = source[source.index(f"def test_no_{privilege}_binaries") :]
        function = function[: function.index("\n\ndef ")]
        assert "2>/dev/null" not in function
        assert "result.returncode == 0" in function
        assert "result.stderr" in function


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


@pytest.mark.parametrize("profile", ("code", "co-work"))
def test_profile_language_dependencies_are_exact_and_lock_derived(profile: str) -> None:
    profile_root = PROJECT_ROOT / "config/profiles" / profile
    python_packages = _source_package_lines(profile_root / "python-requirements.txt")
    npm_packages = _source_package_lines(profile_root / "npm-packages.txt")

    assert all(
        re.fullmatch(r"[A-Za-z0-9_.-]+(?:\[[A-Za-z0-9_.-]+\])?==[^=\s]+", package)
        for package in python_packages
    ), "every direct Python requirement must select one exact version"
    assert all(re.fullmatch(r"@[^/@\s]+/[^@\s]+@[^@\s]+", package) for package in npm_packages), (
        "every direct npm requirement must select one exact version"
    )

    python_lock = profile_root / "python-requirements.lock"
    npm_lock = profile_root / "npm-package-lock.json"
    assert python_lock.is_file(), "the profile must own its hashed Python resolution"
    assert npm_lock.is_file(), "the profile must own its integrity-bound npm resolution"

    locked_python = {
        match.group("name").lower().replace("_", "-")
        for line in python_lock.read_text(encoding="utf-8").splitlines()
        if (match := re.match(r"^(?P<name>[A-Za-z0-9_.-]+)==[^\s\\]+", line))
    }
    direct_python = {
        package.partition("==")[0].lower().replace("_", "-") for package in python_packages
    }
    assert direct_python <= locked_python
    assert "--hash=sha256:" in python_lock.read_text(encoding="utf-8")

    npm_payload = json.loads(npm_lock.read_text(encoding="utf-8"))
    assert npm_payload["lockfileVersion"] == 3
    assert npm_payload["packages"][""]["dependencies"] == {
        package.rsplit("@", 1)[0]: package.rsplit("@", 1)[1] for package in npm_packages
    }
    assert all(
        isinstance(entry.get("integrity"), str) and entry["integrity"].startswith("sha512-")
        for key, entry in npm_payload["packages"].items()
        if key
    )


def _source_package_lines(path: Path) -> tuple[str, ...]:
    return tuple(
        line.strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    )


def test_rootfs_materializer_has_no_floating_bootstrap_or_installer() -> None:
    source = (PROJECT_ROOT / "config/docker/Dockerfile.rootfs-dependencies.j2").read_text(
        encoding="utf-8"
    )
    profile_builds = "\n".join(
        (PROJECT_ROOT / "config/profiles" / profile / "build.sh").read_text(encoding="utf-8")
        for profile in ("code", "co-work")
    )
    combined = source + "\n" + profile_builds

    for forbidden in (
        "nvm-sh/nvm/master",
        "npm@latest",
        "astral.sh/uv/install.sh",
        "--upgrade pip",
        "claude.ai/install.sh",
        "ollama.com/install.sh",
    ):
        assert forbidden not in combined
    assert "uv pip install --system --break-system-packages --require-hashes" in source
    assert "npm ci" in source


def test_rootfs_binary_inputs_are_versioned_and_digest_bound_per_architecture() -> None:
    settings = BUILD.asset_dependencies

    assert set(settings.architectures) == set(BUILD.architectures)
    for arch_name, artifacts in settings.architectures.items():
        assert arch_name in {"arm64", "x86_64"}
        for name in ("node", "uv", "claude", "ollama"):
            artifact = getattr(artifacts, name)
            assert artifact.version in artifact.url
            assert re.fullmatch(r"[0-9a-f]{64}", artifact.sha256)


def test_asset_python_has_no_unclassified_mutable_tool_commands() -> None:
    inventory = _function_string_inventory(
        PROJECT_ROOT / "build_system/builder/image/docker.py"
    )

    # `_rootfs_context` declares commands inserted into the network-open rootfs
    # Dockerfile. `container_compile_agent` is the one sealed compiler client.
    # No post-build packer, probe, inventory, or OBOM helper may grow its own
    # package-manager/fetch path.
    assert set(inventory) == {
        "_rootfs_context",  # declarations rendered into the materializer
        "build_version_script",  # read-only version commands
        "container_compile_agent",  # separately proved locked/offline below
        "extract_software_inventory",  # dpkg-query, pip list, npm ls
        "prepare_build_context",  # copies the npm lock; never invokes npm
        "sync_container_clock",  # pre-materialization Colima clock repair
    }
    # These are read-only inventory commands. Ratchet their executable tool
    # vocabulary directly instead of scanning prose: a docstring such as
    # "installed packages" must not look like an install command.
    assert inventory["extract_software_inventory"] == Counter({"npm": 2, "pip": 1})
    assert inventory["prepare_build_context"] == Counter({"npm": 1})


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
                "npm": 10,
                "uv": 8,
                "apt": 10,
                "apt-get": 3,
                "curl": 2,
                "pip": 1,
                "npx": 1,
            }
        ),
        "config/profiles/co-work/build.sh": Counter({"curl": 3}),
        "config/profiles/code/build.sh": Counter({"curl": 3}),
        "build_system/docker/Dockerfile.asset-tools": Counter({"apt": 5, "curl": 3, "apt-get": 2}),
        "build_system/docker/Dockerfile.guest-rust-builder": Counter({"rustup": 3, "apk": 1, "cargo": 1}),
    }
    candidates = {
        *PROJECT_ROOT.glob("config/docker/**/*.j2"),
        *PROJECT_ROOT.glob("config/profiles/*/build.sh"),
        PROJECT_ROOT / "build_system/docker/Dockerfile.asset-tools",
        PROJECT_ROOT / "build_system/docker/Dockerfile.guest-rust-builder",
    }
    actual = {
        path.relative_to(PROJECT_ROOT).as_posix(): inventory
        for path in sorted(candidates)
        if (inventory := _source_line_inventory(path))
    }
    assert actual == expected


def test_every_debian_asset_materializer_bootstraps_https_trust_before_apt() -> None:
    materializers = (
        BUILD.asset_tools.dockerfile,
        f"config/docker/{BUILD.asset_dependencies.kernel_template}",
        f"config/docker/{BUILD.asset_dependencies.rootfs_template}",
    )

    for relative in materializers:
        source = (PROJECT_ROOT / relative).read_text(encoding="utf-8")
        assert "ARG TRUSTSTORE_IMAGE" in source, relative
        trust = source.index(
            "COPY --from=truststore /etc/ssl/certs/ca-certificates.crt "
            "/etc/ssl/certs/ca-certificates.crt"
        )
        assert trust < source.index("apt-get"), relative


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
    assert ("uv", "sync", "--project", "build_system", "--frozen") in commands
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

    arch = BUILD.architectures["x86_64"]
    changed_arch = arch.model_copy(
        update={"rust_builder_base_image": ("docker.io/library/rust@sha256:" + "f" * 64)}
    )
    changed_build = BUILD.model_copy(
        update={"architectures": {**BUILD.architectures, "x86_64": changed_arch}}
    )
    assert assettools.image_tag(changed_build, "x86_64", PROJECT_ROOT) != assettools.image_tag(
        BUILD, "x86_64", PROJECT_ROOT
    )


def test_asset_tool_helper_replaces_inherited_apt_authority_before_fetch() -> None:
    source = (PROJECT_ROOT / BUILD.asset_tools.dockerfile).read_text(encoding="utf-8")

    trust_stage = source.index("FROM ${TRUSTSTORE_IMAGE} AS truststore")
    trust_copy = source.index(
        "COPY --from=truststore /etc/ssl/certs/ca-certificates.crt "
        "/etc/ssl/certs/ca-certificates.crt"
    )
    remove = source.index("rm -f /etc/apt/sources.list.d/*")
    write = source.index("> /etc/apt/sources.list")
    update = source.index("apt-get -o Acquire::Check-Date=false update")
    assert trust_stage < trust_copy < remove < write < update
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
    assert f"TRUSTSTORE_IMAGE={arch.rust_builder_base_image}" in builds[0]
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
