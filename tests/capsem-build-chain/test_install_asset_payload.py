"""Install package asset-payload contract tests."""

import ast
import errno
import functools
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from types import ModuleType, SimpleNamespace

import pytest
import yaml
from blake3 import blake3

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _skill_text(skill_path: Path) -> str:
    """Read a skill plus the reference files it explicitly links."""
    skill_dir = skill_path.parent
    main = skill_path.read_text(encoding="utf-8")
    parts = [main]
    for relative in dict.fromkeys(re.findall(r"`(references/[A-Za-z0-9_./-]+\.md)`", main)):
        reference = (skill_dir / relative).resolve()
        assert reference.is_relative_to(skill_dir.resolve())
        assert reference.is_file(), f"missing linked skill reference: {relative}"
        parts.append(reference.read_text(encoding="utf-8"))
    return "\n".join(parts)


#: Recipes whose behaviour moved into the gate, and the command that now owns
#: it. These contracts are about what the gate *does*; when the doing moved
#: from a shell body into a plan, the place to read it moved with it.
DISPATCHED = {
    "test-clean:": ("candidate", {}),
    "_test-candidate:": ("test-candidate", {}),
    "_gate-assets:": ("assets", {}),
    "_gate-install:": ("install", {}),
    "_cross-compile": ("cross-compile", {"arch": "arm64"}),
    "_prove-linux-deb:": ("prove-deb", {}),
    "_test-install-harness-preflight:": ("install-image", {}),
    "_docker-gc:": ("storage", {"action": "gc", "rail": None}),
    "_ensure-service:": ("ensure-service", {}),
    "_pack-initrd:": ("pack-initrd", {}),
    "_stamp-version:": ("stamp-version", {}),
    "_gate-host-package-sbom:": ("host-sbom", {}),
    "_gate-linux-rust:": ("linux-rust", {}),
    "_build-assets": ("build-assets", {"profile": "code", "arch": "arm64", "template": "all"}),
    "_check-assets:": ("check-assets", {}),
}


#: Environment names whose values must never reach a test failure's output.
#: The package rail injects the checkout's real Tauri signing key, and a
#: failing assertion prints whatever string it was given -- straight into a CI
#: log. Redacted at the boundary rather than trusted not to fail.
SECRET_ENV = ("TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD")


def _redact(text: str) -> str:
    return re.sub(
        rf"({'|'.join(SECRET_ENV)})=(?:'[^']*'|\S+)",
        r"\1=<redacted>",
        text,
        flags=re.DOTALL,
    )


@pytest.fixture(autouse=True, scope="module")
def _resolvable_package():
    """The built package these contracts plan *around*, made rather than found.

    `_planned` runs the plan against a recording runner to capture real argv,
    so every runtime precondition has to hold or the plan stops at the step
    that needs one. `install` resolves `dist/Capsem_<version>_<arch>.deb` and
    refuses without it, which meant these contracts silently asserted against
    a two-step transcript -- `contextlib.suppress` swallows the refusal, and
    the failure surfaces later as `'chown -R' not in <almost nothing>`.

    On a developer's machine an earlier `just _cross-compile` had left one, so
    this passed for reasons unrelated to what it tests. It failed the moment a
    run got a checkout of its own, which is the same leftover-dependency class
    as the CI run where 94 tests reported no materialized profiles.

    Created only when genuinely absent, and removed again, so a real package is
    never touched and an empty placeholder never outlives the test that needed
    it -- an empty `.deb` left in `dist/` is something a later lane would try
    to install.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.versions import workspace_version

    config = gate_config.load(PROJECT_ROOT)
    package = (
        PROJECT_ROOT
        / "dist"
        / f"Capsem_{workspace_version(PROJECT_ROOT)}_{config.host_arch().dpkg}.deb"
    )
    if package.exists() and package.stat().st_size:
        yield
        return

    package.parent.mkdir(parents=True, exist_ok=True)
    package.write_bytes(b"placeholder: planned, never installed\n")
    try:
        yield
    finally:
        package.unlink(missing_ok=True)


def _planned(command: str, **args) -> str:
    return _planned_cached(command, tuple(sorted(args.items())))


def _selected_content(tmp_path: Path) -> str:
    """A complete paired cohort for executing opaque install-plan callbacks."""
    from capsem.gate import config as gate_config
    from capsem.gate.content import ProfileContent

    config = gate_config.load(PROJECT_ROOT)
    content = ProfileContent.isolated(config, tmp_path / "selected-content")
    inputs = content.root / config.install.selected_inputs_dir
    inputs.mkdir(parents=True)
    selected = inputs / "profile.tar.zst"
    selected.write_bytes(b"immutable selected profile")
    manifest = {
        "assets": {
            "current": "test",
            "releases": {
                "test": {"arches": {name: {} for name in config.architectures}},
            },
        },
        "profiles": {"code": {"url": selected.resolve().as_uri()}},
    }
    payload = json.dumps(manifest).encode()
    (inputs / config.package.release_inputs_name).write_text("{}")
    (inputs / config.install.manifest_name).write_bytes(payload)
    content.assets.mkdir(parents=True)
    (content.assets / config.install.manifest_name).write_bytes(payload)
    for arch in config.architectures:
        (content.assets / arch).mkdir()
    config_manifest = content.config / config.suites.pytest.test_manifest
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_bytes(payload)
    profile = content.profiles(config) / "code/profile.toml"
    profile.parent.mkdir(parents=True)
    profile.write_text("name = 'code'\n")
    return str(content.root)


@functools.cache
def _planned_cached(command: str, args: tuple) -> str:
    """Every command a gate command actually issues, with real argv.

    The plan is *run* against a recording runner rather than merely described.
    Much of this work is still behind `Call`, which renders as prose -- so a
    description would answer "build the install-test image" where the contract
    is about the docker arguments underneath. Running it records those without
    executing anything.
    """
    from helpers.gate import gate_issued

    # The shared reader runs opaque callbacks only in an expendable checkout
    # and marks every action observational. Keeping a private copy of that
    # protocol here let this contract overwrite the source receipt owned by a
    # real gate running the suite.
    return _redact(gate_issued(command, args))


def _just_recipe_block(name: str) -> str:
    """The recipe, and the plan it dispatches to.

    Both, because these contracts predate the extraction and each one is about
    the behaviour rather than about where it is written. A recipe is a
    dispatch now, so reading only its body would answer a question nobody was
    asking; reading only the plan would miss the just-level wiring some of
    these are genuinely about.
    """
    block = _recipe_body(name)
    if name in DISPATCHED:
        command, args = DISPATCHED[name]
        block = block + "\n" + _planned(command, **args)
    return block


def _recipe_body(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(name))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_job_blocks(workflow: str) -> dict[str, str]:
    lines = workflow.splitlines()
    starts: list[tuple[str, int]] = []
    for index, line in enumerate(lines):
        if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
            starts.append((line.strip()[:-1], index))

    blocks: dict[str, str] = {}
    for offset, (name, start) in enumerate(starts):
        end = starts[offset + 1][1] if offset + 1 < len(starts) else len(lines)
        blocks[name] = "\n".join(lines[start:end])
    return blocks


def _load_local_release_glowup() -> ModuleType:
    path = PROJECT_ROOT / "scripts" / "local-release-glowup.py"
    spec = importlib.util.spec_from_file_location("local_release_glowup", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    scripts_path = str(PROJECT_ROOT / "scripts")
    sys.path.insert(0, scripts_path)
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(scripts_path)
        sys.modules.pop(spec.name, None)
    return module


def _load_release_installed_probe() -> ModuleType:
    path = PROJECT_ROOT / "scripts" / "release_installed_probe.py"
    spec = importlib.util.spec_from_file_location("release_installed_probe", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _run_docker_space_gate(
    tmp_path: Path,
    *,
    before_kib: int,
    after_kib: int,
    after_trim_kib: int | None = None,
    volumes: str = "",
    rail: str = "assets",
) -> subprocess.CompletedProcess[str]:
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir(parents=True)
    state = tmp_path / "pruned"
    commands = tmp_path / "docker-commands"
    docker = fake_bin / "docker"
    docker.write_text(
        """#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_COMMANDS"
if [ "$1" = "run" ]; then
    case "$*" in
        *debian:bookworm-slim*)
            phase=$(cat "$FAKE_DOCKER_STATE" 2>/dev/null || true)
            if [ "$phase" = "pruned" ]; then
                free="$FAKE_DOCKER_AFTER_KIB"
            else
                free="$FAKE_DOCKER_BEFORE_KIB"
            fi
            total=$((100 * 1024 * 1024))
            used=$((total - free))
            printf '%s %s %s\\n' "$total" "$used" "$free"
            ;;
        *alpine:3.20*)
            printf 'trimmed\\n' > "$FAKE_DOCKER_STATE"
            ;;
        *)
            printf 'unexpected fake docker run: %s\\n' "$*" >&2
            exit 97
            ;;
    esac
elif [ "$1" = "builder" ] && [ "$2" = "prune" ]; then
    printf 'pruned\\n' > "$FAKE_DOCKER_STATE"
elif [ "$1" = "volume" ] && [ "$2" = "inspect" ]; then
    case " $FAKE_DOCKER_VOLUMES " in
        *" $3 "*) printf '{}\\n' ;;
        *) exit 1 ;;
    esac
elif [ "$1" = "volume" ] && [ "$2" = "rm" ]; then
    :
elif [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
    exit 1
elif [ "$1" = "ps" ]; then
    :
elif [ "$1" = "system" ] && [ "$2" = "df" ]; then
    if [ "$3" = "--format" ]; then
        printf '%s\\n' \
          '{"Active":"0","Reclaimable":"1GB (50%)","Size":"2GB","TotalCount":"2","Type":"Images"}' \
          '{"Active":"0","Reclaimable":"0B","Size":"0B","TotalCount":"0","Type":"Containers"}' \
          '{"Active":"0","Reclaimable":"0B","Size":"0B","TotalCount":"0","Type":"Local Volumes"}' \
          '{"Active":"0","Reclaimable":"8GB","Size":"10GB","TotalCount":"10","Type":"Build Cache"}'
    else
        printf 'Local Volumes space usage:\\n\\nVOLUME NAME LINKS SIZE\\n'
        for volume in $FAKE_DOCKER_VOLUMES; do
            printf '%s 0 1GB\\n' "$volume"
        done
        printf '\\nBuild cache usage:\\n'
    fi
else
    printf 'unexpected fake docker command: %s\\n' "$*" >&2
    exit 97
fi
"""
    )
    docker.chmod(0o755)
    colima = fake_bin / "colima"
    colima.write_text(
        """#!/bin/sh
set -eu
if [ "$1" = "status" ]; then
    printf 'running\\n'
elif [ "$1" = "ssh" ]; then
    printf '/mnt/lima-colima: 1 GiB (1073741824 bytes) trimmed\\n'
else
    exit 97
fi
"""
    )
    colima.chmod(0o755)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{fake_bin}:{env['PATH']}",
            "FAKE_DOCKER_STATE": str(state),
            "FAKE_DOCKER_BEFORE_KIB": str(before_kib),
            "FAKE_DOCKER_AFTER_KIB": str(after_kib),
            "FAKE_DOCKER_VOLUMES": volumes,
            "FAKE_DOCKER_COMMANDS": str(commands),
            "CAPSEM_STORAGE_REPORT_PATH": str(tmp_path / "storage.jsonl"),
        }
    )
    return subprocess.run(
        [
            "bash",
            str(PROJECT_ROOT / "scripts" / "ensure-docker-space.sh"),
            rail,
        ],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )


def _storage_rail(rail: str) -> dict:
    """The storage policy's limits for `rail`.

    Read rather than restated. These fixtures simulate a daemon sitting above
    or below the free-space floor, so every literal here is only meaningful
    relative to that floor: hardcoding "30 GiB is plenty" silently became
    "30 GiB is not enough" the moment the floor moved to 40, and the failure
    surfaced as a release gate refusing to build assets.
    """
    import tomllib

    policy = tomllib.loads(
        (PROJECT_ROOT / "config" / "storage-policy.toml").read_text(encoding="utf-8")
    )
    return policy["rails"][rail]


def _gate_order() -> list[str]:
    """Every step of the complete gate, in an order the graph permits.

    The storage boundaries used to be `_release-*` recipes called from
    `_test-candidate` in a particular line order. They are steps now, so the
    ordering these contracts are about is an edge in one graph rather than a
    sequence spread over a dozen recipe bodies.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    return list(
        GateCommand.registry["candidate"](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        )
        ._describe()
        .labels
    )


def _at(order: list[str], fragment: str) -> int:
    """Where a step sits, by a distinguishing part of its label."""
    for position, label in enumerate(order):
        if fragment in label:
            return position
    raise AssertionError(f"no step matching {fragment!r} in:\n  " + "\n  ".join(order))


def _gate_plan_step(label: str):
    """One step of the complete gate, by label."""
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    return (
        GateCommand.registry["candidate"](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        )
        ._describe()
        .step_named(label)
    )


def _boundary(phase: str) -> str:
    """The boundary a named storage phase releases, from config."""
    from capsem.gate import config as gate_config

    return gate_config.load(PROJECT_ROOT).storage.phases[phase].boundary


def test_asset_gate_owns_docker_capacity_preflight(tmp_path: Path) -> None:
    # The gate refuses to start a build the daemon cannot finish, before the
    # lanes rather than after them -- running out at minute thirty wastes the
    # thirty. The preflight is inside `AssetGate` now; what stays checkable
    # here is that it happens and that the policy it reads is the assets rail.
    assets_source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "assets.py").read_text(
        encoding="utf-8"
    )
    assert "ensure_space" in assets_source
    # The lanes are steps now, so "capacity before building" is an edge rather
    # than statement order: `preflight` reserves and both lanes depend on it.
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_plan

    plan = gate_plan("candidate")
    assert plan.after_of("assets.asset-dependencies") == {"assets.preflight"}
    for arch in ("arm64", "x86_64"):
        assert plan.after_of(f"assets.build.{arch}") == {"assets.asset-dependencies"}

    assets = _storage_rail("assets")
    floor_gib = assets["minimum_free_gib"]
    keep_gib = assets["buildkit_keep_gib"]
    # Comfortably clear of the floor, and clearly under it, whatever it is.
    ample_gib = floor_gib + 10
    starved_gib = max(floor_gib // 4, 1)
    ample_kib = ample_gib * 1024 * 1024
    starved_kib = starved_gib * 1024 * 1024

    enough = _run_docker_space_gate(tmp_path / "enough", before_kib=ample_kib, after_kib=0)
    assert enough.returncode == 0, enough.stderr
    assert "Docker storage control [enforce/preflight]" in enough.stdout

    reclaimed = _run_docker_space_gate(
        tmp_path / "reclaimed",
        before_kib=starved_kib,
        after_kib=ample_kib,
    )
    assert reclaimed.returncode == 0, reclaimed.stderr
    assert "buildkit-pressure-prune" in reclaimed.stdout
    assert f"{starved_gib}.0 GiB -> {ample_gib}.0 GiB" in reclaimed.stdout
    reclaimed_commands = (tmp_path / "reclaimed" / "docker-commands").read_text()
    assert f"builder prune --force --keep-storage {keep_gib}GB" in reclaimed_commands
    assert "builder prune -af" not in reclaimed_commands

    package = _storage_rail("package")
    package_reclaimed = _run_docker_space_gate(
        tmp_path / "package-reclaimed",
        before_kib=starved_kib,
        after_kib=ample_kib,
        rail="package",
    )
    assert package_reclaimed.returncode == 0, package_reclaimed.stderr
    assert f"retain {package['buildkit_keep_gib']} GiB" in package_reclaimed.stdout
    package_commands = (tmp_path / "package-reclaimed" / "docker-commands").read_text()
    assert (
        f"builder prune --force --keep-storage {package['buildkit_keep_gib']}GB" in package_commands
    )

    exhausted = _run_docker_space_gate(
        tmp_path / "exhausted",
        before_kib=starved_kib,
        after_kib=starved_kib,
    )
    assert exhausted.returncode != 0
    assert f"requires {floor_gib}.0 GiB free" in exhausted.stderr

    storage_script = (PROJECT_ROOT / "scripts" / "ensure-docker-space.sh").read_text()
    controller = (PROJECT_ROOT / "scripts" / "docker-storage-policy.py").read_text()
    assert "docker " not in storage_script
    assert '"retained-active"' in controller
    assert '"buildkit-pressure-prune"' in controller


def test_native_install_reuses_the_release_package_builder() -> None:
    from capsem.gate import config as gate_config

    justfile = (PROJECT_ROOT / "justfile").read_text()
    macos_glowup = (PROJECT_ROOT / "scripts" / "macos_release_glowup.py").read_text()
    local_install = (PROJECT_ROOT / "src/capsem/gate/localinstall.py").read_text()
    package_script = gate_config.load(PROJECT_ROOT).install.local_macos_package_script

    assert "\ninstall:" in justfile
    assert "capsem-gate local-install" in justfile
    assert "config.install.local_macos_package_script" in local_install
    assert package_script in macos_glowup
    assert "macos_tart_glowup.py" in macos_glowup
    assert "prove-macos-package-boot.sh" in macos_glowup


def test_cross_compile_repacks_deb_before_exact_systemd_install_proof() -> None:
    """The package is built, repacked against its manifest, validated, and only
    then installed -- and the proof installs the repacked artifact.

    The sequence used to be an inline `bash -c` inside a `docker run` inside a
    recipe, escaped twice over and written as one logical line. It is a
    checked-in script now, syntax-checked with the rest of the shell in the
    repository, so the order is read there.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    script = (PROJECT_ROOT / config.package.build_script).read_text(encoding="utf-8")

    companion = script.index("Build companion host binaries")
    tauri = script.index("cargo tauri build --target")
    repack = script.index("scripts/repack-deb.sh")
    validate = script.index("dpkg-deb --contents")
    assert companion < tauri < repack < validate

    # The manifest url the package is repacked against is the one it will be
    # installed with; never a `file://` pointing at this checkout, which would
    # bake a local path into a publishable package.
    assert "--manifest" in script
    assert "file://$PWD/assets/manifest.json" not in script

    # The package rail decides whether to prove, and hands the proof its
    # arguments rather than exporting three variables and hoping. Two modules
    # since the split: `packagerail` runs the phases, `crosscompile` orders
    # them, and the claim is about the lane rather than about either file.
    rail = "\n".join(
        (PROJECT_ROOT / "src/capsem/gate" / name).read_text(encoding="utf-8")
        for name in ("packagerail.py", "crosscompile.py")
    )
    assert config.package.proof_selector == "scripts/select-linux-deb-proof.sh"
    # The variable is declared in `[package]` and read through it. Asserting
    # the literal appeared in this module was asserting where it was spelled,
    # which stopped being true the moment it got one owner.
    assert config.package.require_proof_variable == "CAPSEM_REQUIRE_LINUX_DEB_PROOF"
    assert "require_proof_variable" in rail
    assert "debproof.DebProof(" in rail
    assert "CAPSEM_PROOF_DEB" not in rail

    # Every packaged binary is present in what was repacked.
    for binary in config.package.proof.binaries:
        assert binary in script, f"{binary} is never validated in the package"

    # The build container hands its outputs back to the host user; nothing is
    # installed inside the builder.
    assert "HOST_UID" in script and "HOST_GID" in script
    assert "dpkg -i" not in script, "the builder installs the package it just built"


def test_exact_linux_deb_proof_uses_systemd_and_proves_guest_shell() -> None:
    """The exact package, installed by dpkg in a real systemd container, and
    then proved by booting a guest shell rather than by checking files exist."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    proof = config.package.proof
    source = (PROJECT_ROOT / "src/capsem/gate/debproof.py").read_text(encoding="utf-8")
    graph = (PROJECT_ROOT / "src/capsem/gate/releasegraph.py").read_text(encoding="utf-8")

    assert config.install.systemd_command == "/usr/lib/systemd/systemd"
    assert config.install.vm_devices == ("/dev/kvm", "/dev/vhost-vsock")
    assert proof.shell_proof_script == "scripts/prove-installed-shell.py"
    assert proof.shell_marker == "CAPSEM_QUALIFIED_DEB_SHELL_OK"
    assert proof.verify_script == "scripts/verify-installed-release.py"

    # The sealed helper already contains every declared dependency. The proof
    # authors and hands over its exact graph, invokes dpkg exactly once, and
    # checks every packaged binary against the package's own version.
    for fragment in ("record_binary", "build_channel", "hand_off"):
        assert fragment in graph
    for fragment in ("author_exact_package", "dpkg", "-i", "dpkg-query"):
        assert fragment in source
    assert "apt-get" not in source
    assert proof.binaries, "no binaries are checked at all"
    for requirement in proof.status_requires:
        assert requirement in source or requirement in str(proof.status_requires)

    # The manifest url and channel arrive as arguments, not as three
    # `CAPSEM_PROOF_*` variables crossing a process boundary that is gone.
    assert "manifest_url" in source and "channel" in source
    assert "os.environ" not in source, (
        "the proof still reads its inputs from the environment rather than taking them as arguments"
    )


def test_systemd_install_image_cannot_flush_host_binfmt_registrations(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A privileged systemd container can remove Colima's Rosetta binfmt entry.

    The damage outlives the run -- every later x86 build on the machine breaks,
    not just the run that caused it -- so the registration is checked before
    the container starts and again after it stops, and the run that removed it
    is the one that reports it.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    dockerfile = (PROJECT_ROOT / config.install.builder.dockerfile).read_text(encoding="utf-8")
    container = (PROJECT_ROOT / "src/capsem/gate/installcontainer.py").read_text(encoding="utf-8")

    assert "/etc/systemd/system/systemd-binfmt.service" in dockerfile
    assert "ln -s /dev/null" in dockerfile

    assert "rosetta_binfmt" in container
    assert "require_rosetta" in container and "verify_rosetta_survived" in container
    assert "removed Colima's Rosetta" in container

    # This is a host-capability check, not an unconditional command in the
    # plan. A Linux dry run must not pretend it will use Colima, while an
    # active macOS Colima gets one probe before and one after the privileged
    # container. Exercise the decision rather than looking for a macOS argv in
    # whatever host happens to be running this suite.
    from helpers.gate import RecordingRunner

    from capsem.gate.installcontainer import InstallContainer

    monkeypatch.setattr("capsem.gate.installcontainer.shutil.which", lambda _name: "/colima")
    for system, machine, expected_checks in (
        ("Linux", "x86_64", 0),
        ("Darwin", "arm64", 2),
    ):
        monkeypatch.setattr("capsem.gate.host.system", lambda system=system: system)
        monkeypatch.setattr("capsem.gate.host.machine", lambda machine=machine: machine)
        runner = RecordingRunner(PROJECT_ROOT)
        install = InstallContainer(runner)

        install.require_rosetta()
        install.verify_rosetta_survived()

        assert len(runner.matching(re.escape(config.install.rosetta_binfmt))) == expected_checks


def test_binary_release_requires_exact_linux_deb_proof() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    native = _workflow_job_blocks(workflow)["test-native-linux-package"]

    assert "build-app-linux:" in workflow
    assert "runs-on: ${{ matrix.runner }}" in workflow
    assert 'python3 scripts/install-deb-runtime-dependencies.py "$package"' in native
    assert 'sudo dpkg -i "$package"' in native
    assert "sudo apt-get install -f -y" not in native
    assert native.index("install-deb-runtime-dependencies.py") < native.index(
        "install-manifest-request.sh write"
    )
    assert "scripts/verify-installed-release.py" in native
    assert "scripts/prove-installed-shell.py" in native
    # Both native runners install and exercise their exact package. GitHub's
    # ARM64 runner has no KVM device, so only the x86_64 row owns the additional
    # guest-shell marker.
    assert "runner: ubuntu-24.04-arm" in native
    assert native.count("if: matrix.arch == 'x86_64'") == 2
    assert "release-exact-shell-x86_64" in native
    assert "just qualify-binaries" in workflow
    assert "just qualify-binaries" in workflow


def test_linux_deb_proof_selector_requires_only_the_native_package() -> None:
    selector = PROJECT_ROOT / "scripts" / "select-linux-deb-proof.sh"

    cases = (
        ("Linux", "x86_64", "x86_64", "1", "1", "prove"),
        ("Linux", "x86_64", "arm64", "0", "1", "skip"),
        ("Linux", "arm64", "arm64", "1", "1", "prove"),
        ("Linux", "arm64", "x86_64", "0", "1", "skip"),
        ("Darwin", "arm64", "arm64", "0", "1", "skip"),
    )
    for host_os, host_arch, target_arch, kvm_ready, required, expected in cases:
        result = subprocess.run(
            [
                "bash",
                str(selector),
                host_os,
                host_arch,
                target_arch,
                kvm_ready,
                required,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == expected


def test_linux_deb_proof_selector_fails_closed_for_native_package_without_kvm() -> None:
    selector = PROJECT_ROOT / "scripts" / "select-linux-deb-proof.sh"

    result = subprocess.run(
        ["bash", str(selector), "Linux", "arm64", "arm64", "0", "1"],
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "native Linux package proof requires KVM and vhost-vsock" in result.stderr


def test_release_matrix_installs_both_architectures_and_uses_available_kvm() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    linux = _workflow_job_blocks(workflow)["test-native-linux-package"]

    assert "runner: ubuntu-24.04-arm" in linux
    assert "runner: ubuntu-24.04" in linux
    assert linux.count("if: matrix.arch == 'x86_64'") == 2
    assert "Enable KVM for exact-package VM proof" in linux
    assert "Prove exact-package guest shell execution" in linux
    assert "CAPSEM_EXACT_PACKAGE_SHELL_OK" in linux
    assert "release-exact-shell-x86_64" in linux


def test_install_test_returns_configured_writable_paths_to_host_identity() -> None:
    """The lifecycle boundary, not the opaque plan, owns path hand-back.

    The install plan now exposes one domain transaction, so looking for its
    internal chown in a plan transcript proves nothing. Drive the production
    container boundary directly and keep the hand-back set config-owned.
    """
    from helpers.gate import RecordingRunner

    from capsem.gate import config as gate_config
    from capsem.gate.installcontainer import InstallContainer

    config = gate_config.load(PROJECT_ROOT)
    runner = RecordingRunner(PROJECT_ROOT)
    InstallContainer(runner).return_paths()
    issued = "\n".join(runner.rendered)
    owned = config.install.layout.owned_paths(config.install.mount)

    assert "chown -R" in issued
    for path in owned:
        assert path in issued, f"{path} is never handed back to the host user"
    # Never the whole mount: a recursive chown of /src walks every cargo
    # artifact in the checkout.
    assert f"chown -R {config.install.mount}" not in issued


def test_install_test_cleanup_preserves_the_original_gate_failure() -> None:
    """The container goes, and the failure that caused it is what propagates.

    The shell captured `install_gate_exit=$?`, disarmed its own trap, removed
    the container, and re-exited with the saved status -- four lines whose
    correctness was their order. `held` gives the same guarantee structurally,
    and `_release` now attaches cleanup failures to the primary error instead
    of replacing it.
    """
    lifecycle = (PROJECT_ROOT / "src" / "capsem" / "gate" / "lifecycle.py").read_text(
        encoding="utf-8"
    )
    install = (PROJECT_ROOT / "src" / "capsem" / "gate" / "install.py").read_text(encoding="utf-8")

    assert "primary" in lifecycle
    assert "add_note" in lifecycle

    # The rail's own teardown order: the handoff is cleared before the
    # container goes, or the next install in this checkout inherits a request
    # pointing at a graph that no longer exists.
    teardown = install.split("finally:", 1)[1]
    assert teardown.index("clear_handoff") < teardown.index("container.stop")


def test_install_test_does_not_rebuild_frontend_and_owns_release_site_scratch() -> None:
    """The release-site scratch is a named volume; the frontend is not rebuilt.

    Named volumes keep the release-site dependencies off the bind-mounted
    checkout and out of a reinstall on every run. The frontend is already built
    by the time this runs, and rebuilding it here would prove a different tree.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    issued = _planned("install")

    # Was: two named volumes, asserted by name and mount point. They are gone.
    # `release-site/node_modules` is baked into the install image, so the
    # runtime `pnpm install --frozen-lockfile` finds a tree that already
    # matches the lockfile; `release-site/dist` is an anonymous volume,
    # allocated per container and reclaimed with it.
    #
    # The property both protected is unchanged and is what is asserted now:
    # the lane does not rebuild the frontend, and nothing it writes lands in a
    # name a second gate could pick up.
    assert "capsem-install-release-site" not in issued, (
        f"a named release-site volume is back: {issued}"
    )
    assert "pnpm run build" not in issued, "the install lane rebuilt the frontend"

    assert "capsem-install-frontend-node-modules" not in issued
    assert "pnpm build" not in issued
    owned = config.install.layout.owned_paths(config.install.mount)
    assert "/src/release-site/node_modules" in owned
    assert "/src/release-site/dist" in owned


def test_install_test_removes_stale_container_before_controller_preflight(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A predecessor is cleared before anything starts, not after it collides."""
    from helpers.gate import RecordingRunner

    from capsem.gate import config as gate_config
    from capsem.gate import installimage
    from capsem.gate.installcontainer import InstallContainer

    config = gate_config.load(PROJECT_ROOT)
    container = config.install.container
    monkeypatch.setattr(installimage, "require_local_image", lambda *_args: config.install.image)
    runner = RecordingRunner(PROJECT_ROOT, replies={"systemctl is-system-running": "running"})
    InstallContainer(runner, sleep=lambda _seconds: None).start(options=[])
    issued = "\n".join(runner.rendered)

    # `-v` since the wrapper takes anonymous volumes with the container it
    # removes; the ordering this test is about is unchanged.
    remove = issued.index(f"docker rm -f -v {container}")
    start = issued.index(f"docker run -d --name {container}")
    assert remove < start


def test_install_test_runs_local_release_glowup_from_real_package() -> None:
    """The glow-up runs against the package this gate installed, not a rebuild."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    proof = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text(encoding="utf-8")

    assert config.install.suite.glowup_script == "scripts/local-release-glowup.py"
    assert config.install.bin_dir == "/usr/bin"
    for flag in ("--input-deb", "--bin-dir", "--package-ready", "--assets-dir", "--config-root"):
        assert flag in proof, f"the glow-up is invoked without {flag}"

    # And the gate still contains it.
    assert "glowup.install" in _gate_order() or "glowup." in " ".join(_gate_order())


def test_install_test_stages_real_profile_assets_for_mandatory_vm_proofs() -> None:
    """The installed product is proved against real assets and a real graph.

    A scratch tree per run, cleared first so a previous failure cannot leave
    half a channel behind; the graph built and checked around the site render,
    in that order, because the check reads what the render produced.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    layout = config.install.layout
    proof = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text(encoding="utf-8")
    graph = (PROJECT_ROOT / "src/capsem/gate/releasegraph.py").read_text(encoding="utf-8")

    assert layout.assets == "target/install-test-assets"
    assert layout.config == "target/install-test-config"
    assert config.install.generated_inputs == ("dist",)
    assert config.install.suite.serve_script == "scripts/serve-release-test-root.py"
    assert "stage_content" in proof
    assert "cmp -s" in proof
    assert "stage-release-test-inputs" not in proof

    # Build the graph, render the site over it, then check it.
    build = graph.index("def build_channel")
    render = graph.index("release-site-build")
    check = graph.index("channel check")
    assert build < render < check

    # The renderer is told which graph to read and where the render goes.
    # Through `[environment.release_site]`, which owns both names -- one used
    # to mean input *and* output, and keeping them adjacent under one owner is
    # what makes the pair readable.
    site = config.environment.release_site
    assert (site.graph, site.channel_dist) == (
        "CAPSEM_RELEASE_GRAPH",
        "CAPSEM_RELEASE_CHANNEL_DIST",
    )
    assert "self._site.graph" in graph
    assert "self._site.channel_dist" in graph


def test_install_test_consumes_exact_publishable_package_without_rebuild() -> None:
    """The package is selected by this checkout's version, not globbed.

    `dist/` accumulates, so a glob would let a package built from a different
    commit be installed and proved. Selecting by version and refusing an empty
    or missing file is what makes "the exact publishable package" true.
    """
    install = (PROJECT_ROOT / "src/capsem/gate/install.py").read_text(encoding="utf-8")

    assert 'f"Capsem_{self.version}_{self.arch.dpkg}.deb"' in install
    assert "missing exact release-mode Debian package" in install
    assert "st_size == 0" in install
    assert "glob(" not in install, "the package is globbed rather than named"


def test_local_release_glowup_uses_real_release_pipeline_not_fake_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    authoring = (PROJECT_ROOT / "src/capsem/gate/releaseauthoring.py").read_text()
    tree = ast.parse(script)
    clone_functions = [
        node
        for node in tree.body
        if isinstance(node, ast.FunctionDef) and node.name == "clone_manifest_for_channel"
    ]

    assert "scripts/repack-deb.sh" in script
    assert "scripts/generate-host-binary-sbom.py" in script
    assert "record-binary" in authoring
    assert '"--source-commit"' in authoring
    # Told, never resolved -- the invariant this line has always protected. The
    # script authors a release graph carrying package provenance, and the tree
    # it sits in is not always the subject: inside the install container it is a
    # mount. What changed is only where "told" may come from. It was
    # `required=True`, and the release lane's two glow-up steps passed nothing,
    # so neither could get past `argparse`; the default is now the commit the
    # gate exports for every action, which is still an answer from the party
    # that knows. An explicit flag continues to win.
    assert '"--source-commit"' in script
    assert "environment.qualified_source_commit" in script
    assert "source_commit = args.source_commit" in script
    assert "source_commit_for_checkout" not in script, (
        "the glow-up must never resolve its own commit from the tree it is in"
    )
    assert any(
        isinstance(node, ast.Name) and node.id == "author_native_candidate"
        for node in ast.walk(tree)
    )
    assert '"assets"' in authoring and '"channel"' in authoring and '"build"' in authoring
    assert len(clone_functions) == 1
    assert not any(isinstance(node, ast.Dict) for node in ast.walk(clone_functions[0])), (
        "channel projection must derive from selected manifest bytes, not a literal fake graph"
    )
    assert "stable-assets-manifest.json" in script
    assert "nightly-assets-manifest.json" in script
    assert "clone_manifest_for_channel(" in script
    assert 'args.assets_dir / "manifest.json",' in script
    assert 'stable_manifest,\n            "stable",' in script
    assert 'clone_manifest_for_channel(stable_manifest, nightly_manifest, "nightly")' in script
    assert "CAPSEM_RELEASE_URL" not in authoring
    assert "release_environment" in authoring
    assert "CAPSEM_RELEASE_CHANNELS_URL=" in script
    assert "update --yes --channel nightly" in script
    assert "update --yes --channel stable" in script
    assert script.count("update --assets --channel nightly") == 1
    assert "corp-escape.log" in script
    assert "update --assets --channel stable" not in script
    transition_gate = (PROJECT_ROOT / "scripts" / "check-public-binary-release.py").read_text()
    fixture_transport = (PROJECT_ROOT / "scripts" / "release_fixture_server.py").read_text()
    assert "run_docker_binary_transition_smoke" in transition_gate
    assert "update --yes --channel nightly" in transition_gate
    assert "update --yes --channel stable" in transition_gate
    assert "serve_release_root" in script
    assert "SimpleHTTPRequestHandler" in fixture_transport
    assert '"Cache-Control", "no-store"' in fixture_transport
    assert "--network=host" not in script


def test_local_release_glowup_has_zstd_extraction_support_in_install_image() -> None:
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    dockerfile = (PROJECT_ROOT / config.install.builder.dockerfile).read_text()

    assert "zstd" in config.install.builder.apt_packages
    assert "APT_PACKAGES" in dockerfile
    assert "materialize-install-os" in dockerfile


def test_install_image_has_one_network_open_materializer_and_no_runtime_repairs() -> None:
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    helper = (PROJECT_ROOT / config.install.builder.dockerfile).read_text(encoding="utf-8")
    image = (PROJECT_ROOT / config.install.dockerfile).read_text(encoding="utf-8")
    image_gate = (PROJECT_ROOT / "src/capsem/gate/installimage.py").read_text(encoding="utf-8")
    install = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text(encoding="utf-8")
    deb = (PROJECT_ROOT / "src/capsem/gate/debproof.py").read_text(encoding="utf-8")
    graph = (PROJECT_ROOT / "src/capsem/gate/releasegraph.py").read_text(encoding="utf-8")

    assert "uv sync --locked --no-install-project" in helper
    assert "pnpm fetch --frozen-lockfile" in helper
    assert "COPY --from=dependency-fetch --chown=capsem:capsem /capsem-deps/pnpm" in helper
    assert "APT_SNAPSHOT_BASE" in helper and "APT_SNAPSHOT_ID" in helper
    assert "org.capsem.install-builder.input-key" in helper
    assert image.splitlines()[0] == "# check=skip=InvalidDefaultArgInFrom"
    assert "ARG BASE" in image and "FROM ${BASE}" in image
    assert "apt-get" not in image
    assert "pnpm install --offline --frozen-lockfile" in image
    assert "uv run" not in image_gate
    assert "uv run" not in install
    assert "apt-get install -f" not in install
    assert "apt-get install -f" not in deb
    assert "pnpm install" not in graph


def test_installed_glowup_uses_the_materialized_python_without_project_sync(
    monkeypatch,
    tmp_path: Path,
) -> None:
    installed_probe = _load_release_installed_probe()
    interpreter = "/opt/capsem venv/bin/python"
    monkeypatch.setattr(installed_probe.sys, "executable", interpreter)

    probe = installed_probe.exact_installed_probe_shell(tmp_path)
    quoted = "'/opt/capsem venv/bin/python'"
    assert f"{quoted} scripts/verify-installed-release.py" in probe
    assert f"{quoted} scripts/run-installed-winterfell.py" in probe
    assert "uv run" not in probe

    source = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text(encoding="utf-8")
    assert "uv run" not in source
    assert "python3" not in "\n".join(source.splitlines()[1:])


def test_install_transaction_does_not_rebuild_the_prequalified_image() -> None:
    install = (PROJECT_ROOT / "src/capsem/gate/install.py").read_text(encoding="utf-8")

    assert "installimage.prepare" not in install


def test_install_recipe_invokes_pytest_as_a_module_inside_container(tmp_path: Path) -> None:
    """`python -m pytest`, in the container's own project environment.

    A bare `pytest` resolves to whatever is first on PATH, which inside this
    container is not the environment the lockfile pins.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    issued = _planned("install", selected_content_root=_selected_content(tmp_path))

    assert f"UV_PROJECT_ENVIRONMENT={config.install.venv}" in issued
    assert f"{config.install.venv}/bin/python -m pytest" in issued
    assert "uv run pytest " not in issued


def test_install_recipe_runs_release_glowup_in_clean_project_environment() -> None:
    """Through the materialized interpreter, never ambient Python or uv sync.

    The interpreter on PATH is whatever the image happens to have; the one the
    lockfile pins is the one the product is tested against.
    """
    proof = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text(encoding="utf-8")

    assert "python3 scripts/local-release-glowup.py" not in proof
    assert "uv run" not in proof
    assert "self._settings.venv_python" in proof


def test_native_packages_make_full_doctor_mock_server_self_contained() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    pkg_postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
    cli = (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()
    mock_server = (PROJECT_ROOT / "crates" / "capsem-mock-server" / "src" / "main.rs").read_text()

    for package_script in (build_pkg, pkg_postinstall, repack_deb, deb_postinst):
        assert "capsem-mock-server" in package_script
    assert "std::env::current_exe()" in cli
    assert cli.index("std::env::current_exe()") < cli.index("std::env::current_dir()")
    assert "#[command(version" in mock_server


def test_native_packages_include_the_release_functional_benchmark() -> None:
    """`capsem-bench-rs` ships in the native packages and is built for them."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    package_paths = [
        "crates/capsem-app/tauri.conf.json",
        "docker/Dockerfile.host-builder",
    ]
    for path in package_paths:
        if (PROJECT_ROOT / path).is_file():
            pass
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    benchmark = (PROJECT_ROOT / "crates/capsem-bench/src/main.rs").read_text()
    build_script = (PROJECT_ROOT / config.package.build_script).read_text()

    assert "capsem-bench-rs" in workflow
    assert "capsem-bench-rs" in build_script, (
        "the packaged cohort no longer builds the release benchmark"
    )
    assert '#[command(version = env!("CARGO_PKG_VERSION")' in benchmark


def test_binary_packages_embed_public_url_but_install_against_serialized_source() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    macos = workflow.split("  build-app-macos:\n", maxsplit=1)[1].split(
        "\n  build-app-linux:\n", maxsplit=1
    )[0]
    linux = workflow.split("  build-app-linux:\n", maxsplit=1)[1].split(
        "\n  author-binary-candidate:\n", maxsplit=1
    )[0]
    native_macos = workflow.split("  test-native-macos-package:\n", maxsplit=1)[1].split(
        "\n  test-native-linux-package:\n", maxsplit=1
    )[0]
    native_linux = workflow.split("  test-native-linux-package:\n", maxsplit=1)[1].split(
        "\n  test-binary-pairing:\n", maxsplit=1
    )[0]

    for job in (macos, linux):
        assert "needs: [preflight, resolve-channel-source]" in job
        assert "name: binary-channel-source" in job

    assert "PREACTIVATION_MANIFEST=file://" in macos
    assert 'CAPSEM_ASSET_MANIFEST="$PREACTIVATION_MANIFEST"' in macos
    assert "target/package-content/assets/manifest.json" in linux
    assert "--content-root target/package-content" in linux
    assert "CAPSEM_INSTALL_MANIFEST_URL:" in linux

    assert macos.count('--manifest "$ASSET_MANIFEST_URL"') == 1
    assert linux.count("CAPSEM_INSTALL_MANIFEST_URL:") == 1
    for job in (native_macos, native_linux):
        assert "binary-channel-candidate" in job
        assert "PREACTIVATION_MANIFEST=file://" in job
        assert "scripts/install-manifest-request.sh write" in job
        assert '--manifest-url "$PREACTIVATION_MANIFEST"' in job
        assert "scripts/install-manifest-request.sh clear" in job
    assert (
        "needs: [test-native-macos-package, test-native-linux-package, test-binary-pairing]"
    ) in workflow


def test_full_gate_runs_fast_checks_before_install_harness_preflight() -> None:
    """Minutes before twenty: the cheap failures come first.

    And the preflight itself proves the clean container can run every tool the
    install gate needs -- a cached layer can satisfy `docker build` and still
    be missing one, which is why the smoke check exists at all.
    """
    order = _gate_order()
    preflight = _planned("install-image")

    assert _at(order, "fast.clippy") < _at(order, "install.materialize")
    assert _at(order, "fast.web.frontend") < _at(order, "install.materialize")
    assert _at(order, "install.materialize") < _at(order, "install.image-build")
    assert _at(order, "install.image-build") < _at(order, "install.image-smoke")

    assert "docker/Dockerfile.install-test" in preflight
    assert "source /src/scripts/doctor-linux.sh" in preflight
    assert "linux_musl_toolchain_available" in preflight
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in preflight
    assert "CAPSEM_TEST_OUTPUT_ROOT=/tmp/capsem-test-output" in preflight
    assert "/home/capsem/.venv-install-test/bin/python -m pytest --version" in preflight
    assert "sudo -n true" in preflight

    # A sealed smoke failure is a materialization defect. It cannot repair
    # itself by rebuilding without the cache or reopening the network.
    image = (PROJECT_ROOT / "src/capsem/gate/installimage.py").read_text(encoding="utf-8")
    assert "no_cache=True" not in image
    assert "cacheless rebuild" not in image


def test_install_source_image_prebuilds_fresh_cli_before_sealed_runtime() -> None:
    """Current-source update tests never compile inside the privileged runtime."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    helper = (PROJECT_ROOT / "docker/Dockerfile.install-builder").read_text()
    source = (PROJECT_ROOT / "docker/Dockerfile.install-test").read_text()
    builder = (PROJECT_ROOT / "src/capsem/gate/installbuilder.py").read_text()
    proof = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text()
    tests = "\n".join(
        (PROJECT_ROOT / path).read_text()
        for path in (
            "tests/capsem-install/conftest.py",
            "tests/capsem-install/test_asset_download.py",
            "tests/capsem-install/test_update.py",
        )
    )
    issued = _planned("install-image")

    assert config.install.source_cli.startswith("/")
    assert config.install.builder.cargo_store.startswith("/")
    assert config.environment.install_proof.source_cli == "CAPSEM_INSTALL_SOURCE_CLI"
    assert 'cargo fetch --locked --target "${RUST_TARGET}"' in helper
    assert "/capsem-deps/cargo/registry ${CARGO_STORE}/registry" in helper
    assert "ENV RUSTUP_AUTO_INSTALL=0" in helper
    assert "ENV CARGO_NET_OFFLINE=true" in helper
    assert "cargo build --locked --offline -p capsem --bin capsem" in source
    assert "RUSTUP_AUTO_INSTALL=0" in source
    assert "COPY --from=source-cli --chmod=0555" in source
    assert config.install.source_cli in issued
    assert 'f"RUST_TARGET={host_arch.rust_target}"' in builder
    assert "**proof.runtime(" in proof
    assert "source_cli=self._settings.source_cli" in proof
    assert '["cargo", "build", "-p", "capsem"]' not in tests


def test_dependency_helpers_verify_installed_rust_without_channel_sync() -> None:
    """A verification probe must not become a mutable Rustup fetch edge."""
    for relative in (
        "docker/Dockerfile.install-builder",
        "docker/Dockerfile.package-builder",
    ):
        source = (PROJECT_ROOT / relative).read_text(encoding="utf-8")
        dependency_stage = source.split("FROM ${BASE}", 2)[1]

        assert "rustup show active-toolchain" not in dependency_stage
        assert "ENV RUSTUP_AUTO_INSTALL=0" in dependency_stage
        assert dependency_stage.index("ENV RUSTUP_AUTO_INSTALL=0") < dependency_stage.index(
            "rustup toolchain list"
        )


def test_install_preflight_does_not_claim_asset_only_cdxgen() -> None:
    """The install rail cannot inherit an asset materializer by accident."""
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()
    asset_tools = (PROJECT_ROOT / "docker/Dockerfile.asset-tools").read_text()
    preflight = _planned("install-image")

    assert "cdxgen" not in host_builder
    assert "cdxgen --version" not in preflight
    assert "CDXGEN_SHA256" in asset_tools
    assert "sha256sum -c -" in asset_tools
    assert "cdxgen --version" in asset_tools


def test_cross_arch_tauri_swap_covers_every_native_dev_package() -> None:
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()

    assert set(config.toolchain.linux.cross_dev_packages) <= set(
        config.toolchain.linux.apt_packages
    )
    assert set(config.toolchain.linux.cross_host_packages) <= set(
        config.toolchain.linux.apt_packages
    )
    assert 'DEV_PACKAGES_RAW="${4:?cross-architecture dev packages are required}"' in swap_script
    assert 'HOST_PACKAGES_RAW="${5:?host-architecture packages are required}"' in swap_script
    assert 'read -r -a DEV_PACKAGES <<< "$DEV_PACKAGES_RAW"' in swap_script
    assert 'read -r -a HOST_PACKAGES <<< "$HOST_PACKAGES_RAW"' in swap_script
    assert "DEV_PACKAGES=(" not in swap_script


def test_cross_arch_tauri_swap_excludes_non_crossable_introspection_toolchain() -> None:
    """Cross builds must not pull foreign executables that require emulation."""
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    # librsvg2-dev depends on gobject-introspection for the target architecture
    # on Ubuntu 24.04. That dependency is a target executable/Python toolchain,
    # is not required to compile Capsem, and cannot run in the native builder.
    assert "librsvg2-dev" not in swap_script
    assert "librsvg2-dev" not in host_builder
    assert "gobject-introspection" not in swap_script
    assert "qemu" not in swap_script.lower()


def test_cross_arch_frontend_fetch_is_isolated_from_the_dev_library_swap() -> None:
    """Both are materialization, in sibling stages, never qualification work."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    script = (PROJECT_ROOT / config.package.build_script).read_text(encoding="utf-8")
    helper = (PROJECT_ROOT / config.package.builder.dockerfile).read_text(encoding="utf-8")

    assert "swap-dev-libs" not in script
    stages = helper.split("FROM ${BASE}")
    assert len(stages) == 3
    fetch, final = stages[1:]
    assert "pnpm fetch --frozen-lockfile" in fetch
    assert "swap-dev-libs" not in fetch
    assert "swap-dev-libs" in final
    assert "pnpm fetch" not in final


def test_cross_compile_reasserts_pinned_rust_target_before_expensive_work() -> None:
    """The toolchain is pinned by the file that pins it, read not repeated.

    It was spelled three times inside one inline shell script -- three chances
    for a bump to leave the package rail behind.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.packageinputs import pinned_toolchain

    config = gate_config.load(PROJECT_ROOT)
    pinned = pinned_toolchain(PROJECT_ROOT)

    assert config.package.toolchain_pin == "rust-toolchain.toml"
    assert pinned == "1.97.1"

    script = (PROJECT_ROOT / config.package.build_script).read_text(encoding="utf-8")
    helper = (PROJECT_ROOT / config.package.builder.dockerfile).read_text(encoding="utf-8")
    assert "rustup show active-toolchain" in script
    assert "rustup target list" in script
    assert "rustup toolchain list" in helper
    assert 'rustup target list --toolchain "${selected}" --installed' in helper
    assert "rustup target add" not in script + helper


def test_deb_repacker_strips_each_elf_with_its_target_tool_and_fails_closed() -> None:
    repack = (PROJECT_ROOT / "scripts/repack-deb.sh").read_text()

    assert "x86_64-linux-gnu-strip" in repack
    assert "aarch64-linux-gnu-strip" in repack
    assert "CAPSEM_REPACK_STRIP" not in repack
    assert "could not be stripped" not in repack


def test_cross_compile_reuses_only_the_exact_host_builder_identity() -> None:
    """A warm retry skips six minutes without accepting a stale Dockerfile."""
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()
    issued = _planned("cross-compile", arch="arm64")

    assert "docker image inspect --format" in issued
    assert "{{.Os}}/{{.Architecture}}" in issued
    assert "docker image inspect --platform" not in issued
    assert "org.capsem.host-builder.input-key" in issued
    assert "docker build -t capsem-host-builder" not in issued
    assert "org.capsem.host-builder.input-key" in host_builder
    assert host_builder.index("COPY swap-dev-libs.sh") > host_builder.index("FROM")


def test_cross_compile_preflights_docker_capacity_after_builder_before_package() -> None:
    """Capacity is checked twice: once the builder exists, once before the build.

    The builder image itself consumes the headroom, so a single check before it
    measures a number that is wrong by the time it matters.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    source = (PROJECT_ROOT / "src/capsem/gate/packagerail.py").read_text()
    plan = _planned("cross-compile", arch="arm64")

    assert source.count('ensure_space("package")') == 2
    assert plan.index("host-image") < plan.index("package.arm64.space")
    assert plan.index("package.arm64.space") < plan.index("package.arm64.materialize")
    assert plan.index("package.arm64.materialize") < plan.index("package.arm64.build")
    assert config.package.builder.runtime_network == "none"


def test_package_boundary_releases_only_completed_docker_rail_volumes() -> None:
    policy = tomllib.loads((PROJECT_ROOT / "config/storage-policy.toml").read_text())

    assert _boundary("completed-docker-rails") == "after-assets"
    resources = policy["resources"]
    # Every one of these is obsolete now. The agent build's target directory
    # and rustup home are anonymous volumes reclaimed with their container, so
    # there is no named resource for a boundary to hand back and no cache for a
    # later run to inherit.
    for name in (
        "capsem-agent-target-arm64",
        "capsem-agent-target-x86_64",
        "capsem-rustup-arm64",
        "capsem-rustup-x86_64",
    ):
        assert resources[name]["retention"] == "obsolete", (
            f"{name} is live again; no lane may mount a named volume"
        )


def test_the_parity_lane_holds_no_build_tree_for_the_assets_to_wait_on(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The parity lane's build tree is gone, so nothing has to hand it back.

    This asserted an ordering -- lane, then `storage.completed-linux-rust-target`,
    then `assets.preflight` -- because an 11 GiB `capsem-linux-rust-target`
    volume survived the lane and the assets ran with its space still held
    unless a step gave it back first. Sealing the lane deleted the mount, so
    the volume, its `after-linux-rust` boundary and the releasing step went too.

    The property is now unconditional rather than ordering-dependent: the
    assets cannot be starved by a tree the lane never holds. Asserting the old
    sequence would only prove the ceremony came back.
    """
    from helpers.gate import gate_plan

    # Native Linux coverage already executes Linux cfg branches. macOS alone
    # needs the sealed Docker parity lane, and the asset phase must wait for
    # it there. Assert both plans explicitly so the contract means the same
    # thing regardless of which host collected it.
    for system, machine, expected in (
        ("Linux", "x86_64", False),
        ("Darwin", "arm64", True),
    ):
        monkeypatch.setattr("capsem.gate.host.system", lambda system=system: system)
        monkeypatch.setattr("capsem.gate.host.machine", lambda machine=machine: machine)
        plan = gate_plan("candidate")

        assert ("linux-rust" in plan.labels) is expected
        assert ("linux-rust" in plan.after_of("assets.preflight")) is expected
        assert not [name for name in plan.labels if "completed-linux-rust-target" in name], (
            "a step exists to release a volume the sealed lane never mounts"
        )

    from capsem.gate import config as gate_config

    boundaries = {
        phase.boundary for phase in gate_config.load(PROJECT_ROOT).storage.phases.values()
    }
    assert "after-linux-rust" not in boundaries


def test_install_boundary_releases_only_completed_package_targets() -> None:
    """Each package's build tree, released once that architecture is done."""
    order = _gate_order()

    assert _boundary("completed-package-arm64") == "after-package-arm64"
    assert _boundary("completed-package-x86_64") == "after-package-x86_64"
    assert (
        _at(order, "package.arm64")
        < _at(order, "storage.completed-package-arm64")
        < _at(order, "glowup.install")
    )


def test_full_gate_releases_deferred_install_target_between_package_arches() -> None:
    """Between the two package builds, not after both.

    The second build needs the headroom the install rail is still reserving.
    """
    order = _gate_order()

    assert _boundary("deferred-install-target") == "before-packages"
    assert (
        _at(order, "package.arm64")
        < _at(order, "storage.deferred-install-target")
        < _at(order, "package.x86_64")
    )


def test_full_gate_releases_completed_buildkit_graph_after_packages() -> None:
    """After the *second* consumer, never between the assets and the assembly.

    `capsem-host-builder` is a dependency of both package builds, so its final
    tag survives until neither needs it.
    """
    order = _gate_order()

    # `after-install`, not `after-packages`: the install helper derives from
    # the exact local host builder before the source image is sealed, so the
    # packages are not the last thing that needs the parent tag.
    import tomllib

    policy = tomllib.loads(
        (PROJECT_ROOT / "config" / "storage-policy.toml").read_text(encoding="utf-8")
    )
    builder = policy["resources"]["capsem-host-builder"]
    assert builder["release_boundary"] == "after-install"
    assert builder["last_consumer"] == "install"
    assert _at(order, "package.arm64") < _at(order, "package.x86_64") < _at(order, "glowup.install")


def test_full_gate_bounds_docker_storage_without_flushing_rebuild_caches() -> None:
    """The whole gate's budget is taken once, up front, and reclaims nothing
    a rebuild would have to redo."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    order = _gate_order()
    plan_source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "candidateplan.py").read_text(
        encoding="utf-8"
    )

    assert "candidate-boundary" not in config.storage.phases
    assert tuple(config.candidate.candidate_budget) == ("default", "candidate-boundary")
    # `candidate-boundary` labels the capacity evidence. It is not a release
    # phase: no working resource is owned before the candidate starts, so a
    # release action here would only take two snapshots and reclaim nothing.
    budget = _gate_plan_step("prepare.storage-budget")
    rendered = budget.render()
    assert any("no room to finish" in line for line in rendered)
    assert not any("release the storage held" in line for line in rendered)
    assert _at(order, "prepare.storage-budget") < _at(order, "assets.preflight")
    for destructive in ("docker image rm -f", "docker volume rm", "docker buildx prune"):
        assert destructive not in plan_source


def test_full_gate_releases_stage_final_images_and_bounds_completed_cache() -> None:
    """The whole storage arc, as one ordering rather than a dozen call sites."""
    from helpers.gate import gate_plan

    from capsem.gate import host

    plan = gate_plan("candidate")
    order = _gate_order()

    assert "storage.install-preflight" not in plan.labels
    static_leaves = {
        "static.guest-binary-contracts",
        "static.sign",
    }
    if host.on_macos():
        static_leaves.add("linux-rust")
    assert static_leaves <= plan.after_of("assets.preflight")
    assert "install.image-smoke" not in plan.after_of("assets.preflight")
    assert ("install.image-smoke", "glowup.install") in plan.edges
    assert ("linux-rust" in plan.labels) is host.on_macos()
    assert (
        _at(order, "assets.preflight")
        < _at(order, "package.arm64")
        < _at(order, "package.x86_64")
        < _at(order, "glowup.install")
    )


def test_docker_gc_reclaims_old_created_debug_containers() -> None:
    controller = (PROJECT_ROOT / "scripts/docker-storage-policy.py").read_text()

    assert "gc" in _planned("storage", action="gc", rail=None)
    assert '"container",\n                    "prune"' in controller
    assert 'f"until={container_age}h"' in controller
    assert "--filter status=exited" not in controller


def test_install_gate_has_no_disposable_compiler_state_before_pytest() -> None:
    """The package is installed, its ledger handed back, and only then tested.

    `/cargo-target` is the builder's disposable state; it must not be visible
    to a proof about an installed product, or the proof is about the build
    tree instead.
    """
    issued = _planned("install")

    assert "/cargo-target" not in issued

    # No apt rail anywhere bypasses repository freshness: a package installed
    # from a repository the client was told to stop checking is not the package
    # the release publishes.
    for script in sorted((PROJECT_ROOT / "scripts").glob("*.sh")):
        assert "Acquire::Check-Valid-Until=false" not in script.read_text(
            encoding="utf-8", errors="ignore"
        ), f"{script.name} disables apt freshness checking"


def test_cross_compile_does_not_bypass_apt_date_validation() -> None:
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()

    assert "Acquire::Check-Valid-Until=false" not in swap_script
    assert "Acquire::Check-Date=false" not in swap_script


def test_cross_compile_apt_sources_are_encrypted_retried_and_fail_closed() -> None:
    sources = (PROJECT_ROOT / "docker/sources-multiarch.sh").read_text()

    assert "${1:?Ubuntu snapshot base is required}" in sources
    assert "${2:?Ubuntu snapshot ID is required}" in sources
    assert "${snapshot_base%/}/${snapshot_id}" in sources
    assert "archive.ubuntu.com" not in sources
    assert "ports.ubuntu.com" not in sources
    assert "security.ubuntu.com" not in sources
    assert 'Acquire::Retries "5";' in sources
    assert 'Acquire::https::Timeout "30";' in sources
    assert 'APT::Update::Error-Mode "any";' in sources


def test_host_builder_bootstraps_https_trust_before_ubuntu_package_fetches() -> None:
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    trust_stage = (
        "FROM alpine:3.22@sha256:"
        "14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce "
        "AS truststore"
    )
    trust_copy = (
        "COPY --from=truststore /etc/ssl/certs/ca-certificates.crt "
        "/etc/ssl/certs/ca-certificates.crt"
    )
    sources_copy = "COPY sources-multiarch.sh /tmp/"
    normalized = re.sub(r"\s+", " ", host_builder)
    first_update = "apt-get update && apt-get install -y --no-install-recommends"
    ubuntu_stage = next(
        line for line in host_builder.splitlines() if line.startswith("FROM ubuntu:24.04")
    )
    assert trust_stage in host_builder
    assert "@sha256:" in ubuntu_stage
    assert host_builder.index(trust_stage) < host_builder.index(ubuntu_stage)
    assert (
        host_builder.index(ubuntu_stage)
        < host_builder.index(trust_copy)
        < host_builder.index(sources_copy)
        < host_builder.index("apt-get update")
    )
    assert first_update in normalized


def test_cross_arch_tauri_swap_refreshes_indexes_before_removing_native_libs() -> None:
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()

    update = swap_script.index("apt-get update -qq")
    remove = swap_script.index('apt-get remove -y "${DEV_PACKAGES[@]}"')
    install = swap_script.index("apt-get install -y --no-install-recommends")
    assert update < remove < install
    assert swap_script.count("apt-get update -qq") == 1


def test_host_builder_uses_shared_apt_authority_without_refetching_for_python() -> None:
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()
    gate = tomllib.loads((PROJECT_ROOT / "config/gate.toml").read_text())
    native_tools = host_builder.split(
        "# ---- Native build tools + cross-compilation toolchains ----", maxsplit=1
    )[1].split("# ---- Node.js 24 + pnpm 10 ----", maxsplit=1)[0]
    python = host_builder.split("# ---- Exact uv binary", maxsplit=1)[1].split(
        "# ---- Helper script", maxsplit=1
    )[0]

    assert "python3" in gate["toolchain"]["linux"]["apt_packages"]
    assert "$WORKSPACE_APT_PACKAGES" in native_tools
    assert "    python3 \\" not in native_tools
    assert "python3-venv \\" in native_tools
    assert native_tools.count("apt-get update") == 1
    assert host_builder.count("apt-get update") == 1
    assert "apt-get update" not in python
    assert "COPY --from=uv-runtime /uv /uvx /usr/local/bin/" in python


def test_host_builder_uses_digest_pinned_prebuilt_node_runtime() -> None:
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    node_stage = (
        "FROM node:24-bookworm-slim@sha256:"
        "6f7b03f7c2c8e2e784dcf9295400527b9b1270fd37b7e9a7285cf83b6951452d "
        "AS node-runtime"
    )
    assert node_stage in host_builder
    assert "COPY --from=node-runtime /usr/local/ /usr/local/" in host_builder
    assert "deb.nodesource.com" not in host_builder


def test_standalone_install_gate_preflights_privileged_helper(tmp_path: Path) -> None:
    """Capacity and the harness image come before the privileged container.

    Proving the clean container can launch its runner takes a minute;
    discovering it cannot after the expensive work wastes far more.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli
    from capsem.gate.command import GateCommand

    assert cli is not None  # importing the command module registers the command
    plan = GateCommand.registry["install"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(
            dry_run=False,
            graph=False,
            timing=False,
            selected_content_root=_selected_content(tmp_path),
        ),
    )._describe()

    assert ("install.capacity", "install.materialize") in plan.edges
    assert ("install.materialize", "install.image-build") in plan.edges
    assert ("install.image-build", "install.image-smoke") in plan.edges
    assert ("install.image-smoke", "install") in plan.edges


def test_install_gate_passes_vm_devices_to_full_installed_proofs() -> None:
    """A host that can boot a guest passes the devices through; one that
    cannot proves packaging and says so rather than quietly proving less."""
    from capsem.gate import config as gate_config
    from capsem.gate.installcontainer import InstallContainer

    config = gate_config.load(PROJECT_ROOT)
    assert config.install.vm_devices == ("/dev/kvm", "/dev/vhost-vsock")
    assert config.install.optional_vm_devices == ("/dev/vsock",)

    container_source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "installcontainer.py").read_text(
        encoding="utf-8"
    )
    assert "seccomp=unconfined" in container_source
    assert "--device" in container_source
    assert InstallContainer is not None


def test_macos_install_gate_consumes_native_full_probe_evidence() -> None:
    """A Mac cannot nest Apple VZ, so it proves the package natively and hands
    the report to the install rail rather than pretending it booted a guest."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    order = _gate_order()
    proof = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text(encoding="utf-8")

    assert config.install.suite.macos_report_check == "scripts/check-macos-native-glowup.py"
    assert "validate_macos_glowup" in proof
    assert "boots_a_guest" in proof

    if "glowup.macos-package" in order:
        assert _at(order, "glowup.macos-package") < _at(order, "glowup.install")


def test_macos_install_gate_missing_native_report_fails_before_cleanup() -> None:
    """A missing report is a refusal, not a silently reduced proof.

    `${VAR:?}` would have failed inside the shell's own expansion, before the
    diagnostic that explains what to do about it.
    """
    proof = (PROJECT_ROOT / "src/capsem/gate/installproof.py").read_text(encoding="utf-8")

    assert "native glow-up report" in proof
    assert "GateError" in proof
    assert ":?" not in proof, "a shell expansion is still standing in for a diagnostic"


def test_binary_release_sbom_jobs_install_zstd_for_deb_payloads() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    jobs = _workflow_job_blocks(workflow)
    author = jobs["author-binary-candidate"]

    assert "Install host SBOM archive deps" in author
    assert "zstd" in author
    assert author.index("Install host SBOM archive deps") < author.index(
        "Generate packaged host SBOM once"
    )
    assert "name: binary-host-sbom" in author
    assert "Generate packaged host SBOM" not in jobs["create-release"]
    assert "Generate packaged host SBOM" not in jobs["assemble-release-channel"]


def test_local_release_glowup_channel_build_uses_local_release_urls() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    authoring = (PROJECT_ROOT / "src/capsem/gate/releaseauthoring.py").read_text()

    assert "CAPSEM_RELEASE_URL" not in authoring
    assert "release_environment" in authoring
    assert "--asset-source-base" in authoring
    assert 'f"{base_url}/assets/releases/{{asset_version}}"' in script
    assert "stage_manifest_artifacts(" in script


def test_local_release_glowup_uses_preserved_admin_binary_without_rebuild() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    authoring = (PROJECT_ROOT / "src/capsem/gate/releaseauthoring.py").read_text()

    assert 'admin = args.bin_dir / "capsem-admin"' in script
    assert "os.access(admin, os.X_OK)" in script
    assert "str(admin)" in authoring
    assert '"cargo"' not in authoring


def test_local_release_glowup_repack_uses_selected_asset_fixture(
    tmp_path: Path,
    monkeypatch,
) -> None:
    glowup = _load_local_release_glowup()
    commands: list[list[str]] = []
    monkeypatch.setattr(glowup, "run", lambda command, **_kwargs: commands.append(command))

    assets_dir = tmp_path / "isolated-assets"
    glowup.repack_deb(
        tmp_path / "input.deb",
        tmp_path / "output.deb",
        tmp_path / "bin",
        tmp_path / "config",
        assets_dir,
        "https://release.invalid/assets/stable/manifest.json",
    )

    assert commands == [
        [
            "bash",
            "scripts/repack-deb.sh",
            "--manifest",
            "https://release.invalid/assets/stable/manifest.json",
            str(tmp_path / "input.deb"),
            str(tmp_path / "bin"),
            str(tmp_path / "config"),
            str(assets_dir),
            str(tmp_path / "output.deb"),
        ]
    ]


def test_local_release_glowup_hardlinks_same_filesystem_immutable_blobs(
    tmp_path: Path,
    monkeypatch,
) -> None:
    """Late release staging must not allocate a second multi-GB asset cohort."""
    glowup = _load_local_release_glowup()
    source = tmp_path / "assets" / "rootfs.erofs"
    target = tmp_path / "dist" / "x86_64-rootfs.erofs"
    source.parent.mkdir()
    source.write_bytes(b"immutable-rootfs-fixture")

    def reject_duplicate_copy(*_args, **_kwargs) -> None:
        raise OSError(errno.ENOSPC, "constrained release runner")

    monkeypatch.setattr(glowup.shutil, "copy2", reject_duplicate_copy)

    glowup.copy_artifact_tree(source, target)

    assert target.read_bytes() == source.read_bytes()
    assert os.path.samefile(source, target)


def test_local_release_glowup_falls_back_to_copy_across_filesystems(
    tmp_path: Path,
    monkeypatch,
) -> None:
    """Hardlink optimization must remain correct when source and dist differ."""
    glowup = _load_local_release_glowup()
    source = tmp_path / "assets" / "rootfs.erofs"
    target = tmp_path / "dist" / "x86_64-rootfs.erofs"
    source.parent.mkdir()
    source.write_bytes(b"cross-filesystem-rootfs-fixture")

    def reject_cross_device_link(*_args, **_kwargs) -> None:
        raise OSError(errno.EXDEV, "cross-device link")

    monkeypatch.setattr(glowup.os, "link", reject_cross_device_link)

    glowup.copy_artifact_tree(source, target)

    assert target.read_bytes() == source.read_bytes()
    assert not os.path.samefile(source, target)


def test_local_release_glowup_stages_package_ready_artifact_into_fresh_tree(
    tmp_path: Path,
) -> None:
    """Reusing a release package must not rely on the repacker creating dirs."""
    glowup = _load_local_release_glowup()
    source = tmp_path / "dist" / "Capsem_1.2.3_arm64.deb"
    target = tmp_path / "work" / "artifacts" / "stable" / "v1.2.3" / source.name
    source.parent.mkdir()
    source.write_bytes(b"exact-publishable-package")

    glowup.stage_package_ready_artifact(source, target)

    assert target.read_bytes() == source.read_bytes()


def test_local_release_glowup_does_not_copy_after_real_disk_exhaustion(
    tmp_path: Path,
    monkeypatch,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "assets" / "rootfs.erofs"
    target = tmp_path / "dist" / "x86_64-rootfs.erofs"
    source.parent.mkdir()
    source.write_bytes(b"disk-exhaustion-rootfs-fixture")

    def reject_full_filesystem(*_args, **_kwargs) -> None:
        raise OSError(errno.ENOSPC, "no free inode or data block")

    copy_attempted = False

    def record_copy_attempt(*_args, **_kwargs) -> None:
        nonlocal copy_attempted
        copy_attempted = True

    monkeypatch.setattr(glowup.os, "link", reject_full_filesystem)
    monkeypatch.setattr(glowup.shutil, "copy2", record_copy_attempt)

    with pytest.raises(OSError, match="no free inode or data block"):
        glowup.copy_artifact_tree(source, target)

    assert not copy_attempted


def test_local_release_glowup_stages_graph_bytes_by_manifest_digest(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "profile.toml"
    source.parent.mkdir()
    payload = b'id = "code"\nrevision = "2030.0101.1"\n'
    source.write_bytes(payload)
    record = {
        "kind": "profile",
        "path": "profiles/code/profile.toml",
        "url": source.resolve().as_uri(),
        "bytes": len(payload),
        "digest": {
            "sha256": hashlib.sha256(payload).hexdigest(),
            "blake3": blake3(payload).hexdigest(),
        },
        "status": "current",
    }
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "packages": [{"status": "current"}],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "config": [record],
                                "images": [],
                                "evidence": [],
                            }
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    dist = tmp_path / "dist"
    base_url = "http://127.0.0.1:43123"

    glowup.stage_manifest_artifacts(manifest_path, tmp_path / "unused", dist, base_url)

    staged = json.loads(manifest_path.read_text(encoding="utf-8"))
    staged_record = staged["profiles"]["code"]["architectures"][0]["config"][0]
    expected_relative = (
        Path("artifacts") / "sha256" / hashlib.sha256(payload).hexdigest() / "profile.toml"
    )
    assert staged_record["url"] == f"{base_url}/{expected_relative.as_posix()}"
    assert (dist / expected_relative).read_bytes() == payload
    assert os.path.samefile(source, dist / expected_relative)


def test_local_release_glowup_projects_only_fully_staged_architectures(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "profile.toml"
    source.parent.mkdir()
    payload = b'id = "code"\nrevision = "2030.0101.1"\n'
    source.write_bytes(payload)

    def record(url: str) -> dict[str, object]:
        return {
            "kind": "profile",
            "path": "profiles/code/profile.toml",
            "url": url,
            "bytes": len(payload),
            "digest": {
                "sha256": hashlib.sha256(payload).hexdigest(),
                "blake3": blake3(payload).hexdigest(),
            },
            "status": "current",
        }

    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "packages": [{"status": "current"}],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "config": [
                                    record(
                                        "/profiles/releases/stable/code/"
                                        "2030.0101.1/arm64/profile.toml"
                                    )
                                ],
                                "images": [],
                                "evidence": [],
                            },
                            {
                                "architecture": "x86_64",
                                "config": [record(source.resolve().as_uri())],
                                "images": [],
                                "evidence": [],
                            },
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    dist = tmp_path / "dist"
    base_url = "http://127.0.0.1:43123"

    glowup.stage_manifest_artifacts(manifest_path, tmp_path / "unused", dist, base_url)

    staged = json.loads(manifest_path.read_text(encoding="utf-8"))
    architectures = staged["profiles"]["code"]["architectures"]
    assert [row["architecture"] for row in architectures] == ["x86_64"]
    staged_record = architectures[0]["config"][0]
    expected_relative = (
        Path("artifacts") / "sha256" / hashlib.sha256(payload).hexdigest() / "profile.toml"
    )
    assert staged_record["url"] == f"{base_url}/{expected_relative.as_posix()}"
    assert (dist / expected_relative).read_bytes() == payload


def test_local_release_glowup_clones_graph_with_only_channel_identity_changed(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "stable-manifest.json"
    destination = tmp_path / "nightly-manifest.json"
    source_manifest = {
        "version": "1.0.143",
        "channel": "stable",
        "packages": [{"name": "Capsem_stable_amd64.deb", "status": "current"}],
        "profiles": {
            "code": {
                "status": "current",
                "revision": "1.0.0",
                "architectures": [{"architecture": "x86_64"}],
            }
        },
    }
    source.write_text(
        json.dumps(source_manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    original = source.read_bytes()

    glowup.clone_manifest_for_channel(source, destination, "nightly")

    assert source.read_bytes() == original
    cloned = json.loads(destination.read_text(encoding="utf-8"))
    expected = dict(source_manifest)
    expected["channel"] = "nightly"
    assert cloned == expected


def test_local_release_glowup_projects_both_switch_channels_from_any_candidate() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    setup = script.split('stable_manifest = manifests / "stable-assets-manifest.json"', maxsplit=1)[
        1
    ].split("record_binary(", maxsplit=1)[0]

    assert 'args.assets_dir / "manifest.json"' in setup
    assert "stable_manifest," in setup
    assert '"stable"' in setup
    assert 'clone_manifest_for_channel(stable_manifest, nightly_manifest, "nightly")' in setup
    assert 'shutil.copy2(args.assets_dir / "manifest.json"' not in setup

    # Nightly must be projected from the *staged* stable manifest. Staging drops
    # architectures whose blobs are absent locally -- ordinary CI pulls one
    # architecture's profile inputs -- so cloning first left nightly describing
    # an unstaged architecture whose URLs still pointed at GitHub, which the
    # hermetic channel then rejected as "not local".
    assert setup.index("stage_manifest_artifacts(stable_manifest") < setup.index(
        'clone_manifest_for_channel(stable_manifest, nightly_manifest, "nightly")'
    )


def test_local_release_glowup_rejects_partially_staged_architecture(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "profile.toml"
    source.parent.mkdir()
    payload = b'id = "code"\n'
    source.write_bytes(payload)

    def record(kind: str, url: str) -> dict[str, object]:
        return {
            "kind": kind,
            "path": f"profiles/code/{kind}.toml",
            "url": url,
            "bytes": len(payload),
            "digest": {
                "sha256": hashlib.sha256(payload).hexdigest(),
                "blake3": blake3(payload).hexdigest(),
            },
            "status": "current",
        }

    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "packages": [{"status": "current"}],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [
                            {
                                "architecture": "x86_64",
                                "config": [record("profile", source.resolve().as_uri())],
                                "images": [
                                    record(
                                        "rootfs",
                                        "/profiles/releases/stable/code/"
                                        "2030.0101.1/x86_64/rootfs.erofs",
                                    )
                                ],
                                "evidence": [],
                            }
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(SystemExit, match="mixes staged and unstaged artifacts"):
        glowup.stage_manifest_artifacts(
            manifest_path,
            tmp_path / "unused",
            tmp_path / "dist",
            "http://127.0.0.1:43123",
        )


def test_local_release_glowup_rejects_graph_bytes_not_matching_manifest(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "rootfs.erofs"
    source.parent.mkdir()
    payload = b"corrupt-rootfs"
    source.write_bytes(payload)
    record = {
        "kind": "rootfs",
        "name": "rootfs.erofs",
        "url": source.resolve().as_uri(),
        "bytes": len(payload),
        "digest": {
            "sha256": "0" * 64,
            "blake3": blake3(payload).hexdigest(),
        },
        "status": "current",
    }
    manifest_path = tmp_path / "manifest.json"
    original = json.dumps(
        {
            "packages": [{"status": "current"}],
            "profiles": {
                "code": {
                    "status": "current",
                    "architectures": [
                        {
                            "architecture": "arm64",
                            "config": [],
                            "images": [record],
                            "evidence": [],
                        }
                    ],
                }
            },
        }
    )
    manifest_path.write_text(original, encoding="utf-8")

    with pytest.raises(SystemExit, match="SHA-256 mismatch"):
        glowup.stage_manifest_artifacts(
            manifest_path,
            tmp_path / "unused",
            tmp_path / "dist",
            "http://127.0.0.1:43123",
        )

    assert manifest_path.read_text(encoding="utf-8") == original


def test_local_release_glowup_reports_capacity_before_late_asset_staging(
    tmp_path: Path,
    monkeypatch,
    capsys,
) -> None:
    glowup = _load_local_release_glowup()
    gib = 1024**3
    monkeypatch.setattr(
        glowup.shutil,
        "disk_usage",
        lambda _path: SimpleNamespace(total=20 * gib, used=8 * gib, free=12 * gib),
    )

    glowup.report_disk_capacity(tmp_path, "before immutable VM blob staging")

    assert capsys.readouterr().out == (
        "Disk capacity (before immutable VM blob staging): 12.0 GiB free of 20.0 GiB\n"
    )


def test_release_site_overlay_replaces_partial_files_without_clobbering_artifacts(
    tmp_path: Path,
) -> None:
    site = tmp_path / "release-site"
    source = site / "dist"
    source.joinpath("profiles", "code").mkdir(parents=True)
    source.joinpath("index.html").write_text("complete-index", encoding="utf-8")
    source.joinpath("profiles", "code", "index.html").write_text(
        "complete-profile",
        encoding="utf-8",
    )
    target = tmp_path / "release-channel"
    target.joinpath("profiles", "releases").mkdir(parents=True)
    immutable = target / "profiles" / "releases" / "profile.toml"
    immutable.write_text("immutable-profile-artifact", encoding="utf-8")
    stale = target / "index.html"
    stale.write_text("", encoding="utf-8")
    stale.chmod(0o200)

    result = subprocess.run(
        [
            "node",
            str(PROJECT_ROOT / "release-site" / "scripts" / "overlay-dist.mjs"),
        ],
        cwd=site,
        env={**os.environ, "CAPSEM_RELEASE_CHANNEL_DIST": str(target)},
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert stale.read_text(encoding="utf-8") == "complete-index"
    assert (
        target.joinpath("profiles", "code", "index.html").read_text(encoding="utf-8")
        == "complete-profile"
    )
    assert immutable.read_text(encoding="utf-8") == "immutable-profile-artifact"


def test_release_skills_require_space_efficient_immutable_staging() -> None:
    for skill_path in (
        PROJECT_ROOT / "skills" / "dev-testing" / "SKILL.md",
        PROJECT_ROOT / "skills" / "release-process" / "SKILL.md",
    ):
        skill = _skill_text(skill_path)
        assert "hardlink-first" in skill
        assert "same-filesystem" in skill
        assert "cross-filesystem" in skill
        assert "constrained-disk" in skill


def test_local_release_glowup_uses_the_safe_manifest_url_resolver() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    checker = script.split("def check_generated_release(", maxsplit=1)[1].split(
        "\ndef release_asset_urls", maxsplit=1
    )[0]
    resolver = script.split("def local_release_artifact_path(", maxsplit=1)[1].split(
        "\ndef release_asset_urls", maxsplit=1
    )[0]

    assert "local_release_artifact_path(base_url, url, dist)" in checker
    assert "safe_relative(" in resolver
    assert "unquote(parsed.path)" in resolver


def test_local_release_glowup_validates_vm_asset_blobs_are_served() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "release_asset_urls" in script
    assert "release is missing VM asset blob" in script
    assert '"artifacts") / "sha256"' in script
    assert "verify_payload(" in script


def test_local_release_glowup_preflights_stable_and_nightly_manifests() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "stable_artifact = check_generated_release(" in script
    assert "nightly_artifact = check_generated_release(" in script
    assert "expected_version=stable_version" in script
    assert "expected_version=nightly_version" in script
    assert "assert_manifest_artifact(manifest, artifact)" in script


def test_local_release_glowup_generated_release_checker_rejects_missing_asset_blob(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        manifest_path = dist / "assets" / "stable" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            f"""{{
  "packages": [
    {{
      "name": "Capsem_1.5.1_amd64.deb",
      "url": "{base_url}/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
    }}
  ],
  "profiles": {{
    "co-work": {{
      "architectures": [
        {{
          "images": [
            {{"url": "{base_url}/assets/releases/2026.0709.13/x86_64-rootfs.erofs"}}
          ],
          "evidence": [
            {{"url": "{base_url}/assets/releases/2026.0709.13/obom.cdx.json"}}
          ]
        }}
      ]
    }}
  }}
}}
""",
            encoding="utf-8",
        )

        try:
            glowup.check_generated_release(
                base_url,
                f"{base_url}/assets/stable/manifest.json",
                deb,
                dist,
                "stable",
            )
        except SystemExit as error:
            assert "generated stable release is missing VM asset blob" in str(error)
            assert "x86_64-rootfs.erofs" in str(error)
        else:
            raise AssertionError("missing VM asset blob was accepted")


def test_local_release_glowup_generated_release_checker_rejects_tampered_blob(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        expected = b"verified-rootfs"
        artifact_path = (
            dist / "artifacts/sha256" / hashlib.sha256(expected).hexdigest() / "rootfs.erofs"
        )
        artifact_path.parent.mkdir(parents=True)
        artifact_path.write_bytes(b"tampered-rootfs")
        manifest_path = dist / "assets" / "stable" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            json.dumps(
                {
                    "packages": [
                        {
                            "name": "Capsem_1.5.1_amd64.deb",
                            "url": (f"{base_url}/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"),
                        }
                    ],
                    "profiles": {
                        "code": {
                            "architectures": [
                                {
                                    "images": [
                                        {
                                            "kind": "rootfs",
                                            "name": "rootfs.erofs",
                                            "url": f"{base_url}/{artifact_path.relative_to(dist)}",
                                            "bytes": len(expected),
                                            "digest": {
                                                "sha256": hashlib.sha256(expected).hexdigest(),
                                                "blake3": blake3(expected).hexdigest(),
                                            },
                                        }
                                    ],
                                    "config": [],
                                    "evidence": [],
                                }
                            ]
                        }
                    },
                }
            ),
            encoding="utf-8",
        )

        with pytest.raises(SystemExit, match=r"SHA-256 mismatch|byte size mismatch"):
            glowup.check_generated_release(
                base_url,
                f"{base_url}/assets/stable/manifest.json",
                deb,
                dist,
                "stable",
            )


def test_local_release_glowup_generated_release_checker_accepts_manifest_root_relative_assets(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        payload = b"fixture"
        for relative in (
            "profiles/releases/nightly/co-work/2026.0709.13/x86_64/profile.toml",
            "profiles/releases/nightly/co-work/2026.0709.13/x86_64/rootfs.erofs",
            "profiles/releases/nightly/co-work/2026.0709.13/x86_64/obom.cdx.json",
        ):
            target = dist / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(payload)
        manifest_path = dist / "assets" / "nightly" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            json.dumps(
                {
                    "packages": [
                        {
                            "name": "Capsem_1.5.1_amd64.deb",
                            "url": (f"{base_url}/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"),
                        }
                    ],
                    "profiles": {
                        "co-work": {
                            "architectures": [
                                {
                                    "images": [
                                        {
                                            "kind": "rootfs",
                                            "name": "rootfs.erofs",
                                            "url": (
                                                "/profiles/releases/nightly/co-work/"
                                                "2026.0709.13/x86_64/rootfs.erofs"
                                            ),
                                            "bytes": len(payload),
                                            "digest": {
                                                "sha256": hashlib.sha256(payload).hexdigest(),
                                                "blake3": blake3(payload).hexdigest(),
                                            },
                                        }
                                    ],
                                    "config": [
                                        {
                                            "kind": "profile",
                                            "path": "profiles/co-work/profile.toml",
                                            "url": (
                                                "/profiles/releases/nightly/co-work/"
                                                "2026.0709.13/x86_64/profile.toml"
                                            ),
                                            "bytes": len(payload),
                                            "digest": {
                                                "sha256": hashlib.sha256(payload).hexdigest(),
                                                "blake3": blake3(payload).hexdigest(),
                                            },
                                        }
                                    ],
                                    "evidence": [
                                        {
                                            "url": (
                                                "/profiles/releases/nightly/co-work/"
                                                "2026.0709.13/x86_64/obom.cdx.json"
                                            ),
                                            "bytes": len(payload),
                                            "digest": {
                                                "sha256": hashlib.sha256(payload).hexdigest(),
                                                "blake3": blake3(payload).hexdigest(),
                                            },
                                        }
                                    ],
                                }
                            ]
                        }
                    },
                }
            ),
            encoding="utf-8",
        )

        glowup.check_generated_release(
            base_url,
            f"{base_url}/assets/nightly/manifest.json",
            deb,
            dist,
            "nightly",
        )


@pytest.mark.parametrize(
    "url",
    [
        "/profiles/releases/%2e%2e/escape",
        "//attacker.invalid/rootfs.erofs",
        "https://attacker.invalid/rootfs.erofs",
        "/profiles/releases/rootfs.erofs?replacement=1",
    ],
)
def test_local_release_glowup_rejects_unsafe_or_nonlocal_manifest_urls(
    tmp_path: Path,
    url: str,
) -> None:
    glowup = _load_local_release_glowup()

    with pytest.raises(SystemExit):
        glowup.local_release_artifact_path(
            "http://127.0.0.1:43123",
            url,
            tmp_path,
        )


def test_local_release_glowup_installed_path_asserts_channel_round_trip_and_provenance(
    monkeypatch,
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    calls: list[list[str]] = []

    monkeypatch.setattr(glowup, "run", lambda cmd, **_kwargs: calls.append(cmd))

    glowup.run_installed_glowup(
        install_script_url="http://127.0.0.1:1234/install.sh",
        release_base_url="http://127.0.0.1:1234",
        stable_manifest_url="http://127.0.0.1:1234/assets/stable/manifest.json",
        nightly_manifest_url="http://127.0.0.1:1234/assets/nightly/manifest.json",
        corp_manifest_url="http://127.0.0.1:1234/corp/manifest.json",
        package_version="1.5.100",
        stable_package=tmp_path / "Capsem_1.5.100_amd64.deb",
        nightly_package=tmp_path / "Capsem_1.5.100_amd64.deb",
        package_architecture="amd64",
    )

    assert len(calls) == 1
    script = calls[0][-1]
    subprocess.run(["bash", "-n"], input=script, text=True, check=True)
    assert 'grep -F \'"package_version": "1.5.100"\'' in script
    assert 'stable_manifest_sha=$(sha256sum "$HOME/.capsem/assets/manifest.json"' in script
    assert 'test "$stable_manifest_sha" = "$stable_manifest_sha_after_switch"' in script
    assert (
        "check_update_log asset_update_complete http://127.0.0.1:1234/assets/nightly/manifest.json"
        in script
    )
    assert 'CAPSEM_RELEASE_CHANNELS_URL="$release_channels_url"' in script
    assert "binary_update_failed" not in script
    assert "binary_update_complete" not in script
    assert "update --yes --channel nightly" in script
    assert "update --yes --channel stable" in script
    assert script.count("update --assets --channel") == 1
    assert 'update --assets --channel nightly > "$HOME/.capsem/corp-escape.log"' in script
    assert '"package_version": "1.5.101"' not in script
    assert "probe_installed_transition fresh-stable" in script
    assert "probe_installed_transition channel-nightly" in script
    assert "probe_installed_transition channel-stable-return" in script
    assert "probe_installed_transition corporate" in script
    assert "probe_installed_transition final-nightly" in script
    assert '"$CAPSEM_BIN" status' in script
    assert 'grep -Fq "Installed: true"' in script
    assert 'grep -Fq "Running:   true"' in script
    assert 'grep -Fq "Service:   ok"' in script
    assert 'grep -Fq "Gateway:   ok"' in script
    assert "scripts/verify-installed-release.py" in script
    assert '"$CAPSEM_BIN" doctor' in script
    assert "scripts/run-installed-winterfell.py" in script
    assert "service status" not in script
    assert "CAPSEM_CHANNEL=nightly" in script
    assert "http://127.0.0.1:1234/corp/manifest.json" in script
    assert (
        "check_update_log asset_update_complete http://127.0.0.1:1234/corp/manifest.json" in script
    )
    assert "corporate channel is locked" in script
    assert "corp_escape_status" in script


def test_local_release_glowup_asserts_channel_isolation_and_corp_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "stable_channel_sha_before_nightly" in script
    assert "nightly channel build mutated stable manifest" in script
    assert "nightly channel build mutated stable package records" in script
    assert 'corp_manifest_url = f"{base_url}/corp/manifest.json"' in script
    assert 'corp_dir = dist / "corp"' in script
    assert "update --yes --channel nightly" in script
    assert "update --yes --channel stable" in script
    assert "check_origin_channel corp" in script


def test_local_release_glowup_forbids_metadata_only_binary_cohorts() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "rewrite_deb_version" not in script
    assert "next_patch_version" not in script
    assert "without recompiling a second binary cohort" not in script


def test_native_glowup_owns_exact_manifest_and_installed_shell_evidence() -> None:
    macos = (PROJECT_ROOT / "scripts" / "macos_release_glowup.py").read_text()
    linux = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    installed_probe = (PROJECT_ROOT / "scripts" / "release_installed_probe.py").read_text()
    authoring = (PROJECT_ROOT / "src/capsem/gate/releaseauthoring.py").read_text()

    assert "assert_manifest_artifact" in macos
    assert "assert_manifest_artifact" in linux
    assert "prove-macos-package-boot.sh" in macos
    assert "exact_installed_probe_shell" in linux
    assert "verify-installed-release.py" in installed_probe
    assert '"--source-commit"' in authoring
    assert "source_commit_for_checkout(ROOT)" in macos


def test_every_native_glowup_uses_graph_first_binary_authoring() -> None:
    """Linux and macOS must not stamp provenance into a legacy projection."""
    gate = (PROJECT_ROOT / "src/capsem/gate/releasegraph.py").read_text()
    linux = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    macos = (PROJECT_ROOT / "scripts" / "macos_release_glowup.py").read_text()

    assert "author_binary_graph(" in gate
    for source in (linux, macos):
        assert any(
            isinstance(node, ast.Name) and node.id == "author_native_candidate"
            for node in ast.walk(ast.parse(source))
        )


def test_dev_service_does_not_replace_installed_assets_with_worktree_symlink() -> None:
    """The dev service syncs assets into its home; it does not symlink the
    worktree over an installed tree, which made the two indistinguishable."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    service = (PROJECT_ROOT / "src/capsem/gate/service.py").read_text(encoding="utf-8")

    assert config.service.sync_assets_script.endswith("sync-dev-assets.sh")
    assert "sync_assets_script" in service
    assert "symlink_to" not in service
    assert config.service.retired_config, "nothing retires the old config layout"
    assert "retired_config" in service


def test_installers_remove_retired_user_and_service_config_rails() -> None:
    scripts = [
        PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall",
        PROJECT_ROOT / "scripts" / "deb-postinst.sh",
        PROJECT_ROOT / "scripts" / "simulate-install.sh",
    ]

    for path in scripts:
        text = path.read_text()
        assert 'retired_user_config="user"".toml"' in text
        assert '"$CAPSEM_DIR/service.toml"' in text or '"$CAPSEM_HOME_DIR/service.toml"' in text
        assert "retired_config_removed" in text


def test_installers_remove_retired_python_admin_bundle() -> None:
    scripts = [
        PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall",
        PROJECT_ROOT / "scripts" / "deb-postinst.sh",
        PROJECT_ROOT / "scripts" / "simulate-install.sh",
    ]

    for path in scripts:
        text = path.read_text()
        assert "capsem-admin-python" in text
        assert "retired_python_admin_bundle_removed" in text


def test_native_postinstall_merges_fresh_check_into_manifest_metadata() -> None:
    for relative in ("scripts/pkg-scripts/postinstall", "scripts/deb-postinst.sh"):
        script = (PROJECT_ROOT / relative).read_text()
        metadata = script.index("manifest-metadata.json")
        hydrate = script.index('update --assets --manifest \\"$MANIFEST_SOURCE\\"')
        refresh = script.index("update --check", hydrate)

        assert metadata < hydrate < refresh, relative
        assert "CAPSEM_RELEASE_MANIFEST_URL" not in script[refresh - 240 : refresh], relative
        assert "update-check.json" not in script, relative
        assert "update-checks" not in script, relative
        assert "update_status_refreshed" in script[refresh:], relative


def test_manifest_generation_public_path_is_capsem_admin() -> None:
    """One authority generates manifests, and the docs say the same one."""
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    initrd = (PROJECT_ROOT / "src/capsem/gate/initrd.py").read_text(encoding="utf-8")

    assert "manifest" in " ".join(config.initrd.manifest)
    assert "capsem-admin" in " ".join(config.initrd.manifest)
    assert "scripts/gen_manifest.py" not in initrd

    public_docs = [
        path
        for path in (PROJECT_ROOT / "docs").rglob("*.md")
        if "manifest generate" in path.read_text(encoding="utf-8", errors="ignore")
    ]
    for path in public_docs:
        text = path.read_text(encoding="utf-8")
        assert "capsem-admin manifest generate" in text
        assert "scripts/gen_manifest.py" not in text


def test_package_builders_stage_manifest_only_not_vm_asset_payload() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
    pkg_preinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "preinstall").read_text()
    pkg_postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()
    pkg_install_user = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "install-user").read_text()

    assert "CAPSEM_PKG_ASSET_MODE" not in build_pkg
    assert "ASSET_MODE=" not in build_pkg
    assert "export COPYFILE_DISABLE=1" in build_pkg
    assert "--manifest" in build_pkg
    assert 'MANIFEST_PATH="${2:?--manifest requires a URL}"' in build_pkg
    assert "materialize_manifest_input" not in build_pkg
    assert "materialize-package-manifest.py" not in build_pkg
    assert 'parsed.scheme not in ("http", "https", "file")' in build_pkg
    assert "urllib.request.Request(" not in build_pkg
    assert "CapsemReleaseValidator/1.0" not in build_pkg
    assert "urllib.request.urlopen" not in build_pkg
    assert "manifest must be a URL" in build_pkg
    assert "pathlib.Path(source).read_bytes()" not in build_pkg
    assert '--version "$VERSION"' in build_pkg
    assert "PKG_VERSION" not in build_pkg
    assert (
        'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' not in build_pkg
    )
    assert (
        'install -m 0644 "$ASSETS_VIEW/manifest.json" "$SHARE_DIR/assets/manifest.json"'
        not in build_pkg
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in build_pkg
    assert (
        'write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$VERSION" "$SHARE_DIR/assets/manifest-metadata.json"'
        in build_pkg
    )
    assert "snapshot_sha256" not in build_pkg
    assert "materialize_manifest_assets" not in build_pkg
    assert "Added asset:" not in build_pkg
    assert "rootfs-" not in build_pkg
    assert "initrd-" not in build_pkg
    assert "vmlinuz-" not in build_pkg
    assert "obom-" not in build_pkg
    assert "sync-dev-assets.sh" not in build_pkg
    assert 'CONFIG_ROOT="${POSITIONAL[3]}"' in build_pkg
    assert 'ditto --norsrc --noextattr "$src" "$dst"' in build_pkg
    assert 'copy_tree_clean "$CONFIG_ROOT/profiles" "$SHARE_DIR/profiles"' in build_pkg
    assert "for package_script in preinstall postinstall install-diagnostics install-user" in build_pkg
    assert 'install -m 0755 "$SCRIPT_DIR/pkg-scripts/$package_script"' in build_pkg
    assert 'xattr -rc "$WORK_DIR/payload" "$PKG_SCRIPTS"' in build_pkg
    assert 'find "$WORK_DIR/payload" "$PKG_SCRIPTS" -name' in build_pkg
    assert '--scripts "$PKG_SCRIPTS"' in build_pkg
    assert "--filter '/\\._[^/]*$'" in build_pkg
    assert "capsem-admin" in build_pkg
    assert "capsem-tui" in build_pkg
    assert "rm -rf /Applications/Capsem.app" in pkg_preinstall
    assert "event=remove_user_app_payload" in pkg_preinstall
    assert 'rm -rf "$USER_HOME/Applications/Capsem.app"' in pkg_preinstall
    assert "rm -rf /usr/local/share/capsem" in pkg_preinstall
    assert "pkill -9 -x capsem-app" in pkg_preinstall
    assert "capsem stop" not in pkg_preinstall
    assert "$CAPSEM_DIR/bin/capsem" not in pkg_preinstall
    assert "event=stop_existing_service" not in pkg_preinstall
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in pkg_preinstall
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in pkg_preinstall
    assert "install-current-run" in pkg_preinstall
    assert "install-latest.log" in pkg_preinstall
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in pkg_preinstall
    assert 'source "$(dirname "$0")/install-user"' in pkg_preinstall
    assert "capsem_resolve_install_user" in pkg_preinstall

    assert "CAPSEM_DEB_ASSET_MODE" not in repack_deb
    assert "ASSET_MODE=" not in repack_deb
    assert "export COPYFILE_DISABLE=1" in repack_deb
    assert "strip_packaged_binaries" in repack_deb
    assert "CAPSEM_REPACK_STRIP" not in repack_deb
    assert '"$strip_tool" --strip-unneeded "$path"' in repack_deb
    assert 'CONFIG_ROOT="${POSITIONAL[2]}"' in repack_deb
    assert "--manifest" in repack_deb
    assert "materialize_manifest_input" not in repack_deb
    assert "materialize-package-manifest.py" not in repack_deb
    assert 'parsed.scheme not in ("http", "https", "file")' in repack_deb
    assert "urllib.request.Request(" not in repack_deb
    assert "CapsemReleaseValidator/1.0" not in repack_deb
    assert "urllib.request.urlopen" not in repack_deb
    assert "manifest must be a URL" in repack_deb
    assert "pathlib.Path(source).read_bytes()" not in repack_deb
    assert "BUILD_TS=" not in repack_deb
    assert (
        'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' not in repack_deb
    )
    assert (
        'cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"'
        not in repack_deb
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in repack_deb
    assert 'PACKAGE_VERSION="$(dpkg-deb -f "$INPUT_DEB" Version)"' in repack_deb
    assert (
        'write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$PACKAGE_VERSION" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-metadata.json"'
        in repack_deb
    )
    assert "snapshot_sha256" not in repack_deb
    assert "materialize_manifest_assets" not in repack_deb
    assert "Added asset:" not in repack_deb
    assert "rootfs-" not in repack_deb
    assert "initrd-" not in repack_deb
    assert "vmlinuz-" not in repack_deb
    assert "obom-" not in repack_deb
    assert (
        'cp -R "$CONFIG_ROOT/profiles/." "$WORK_DIR/deb/usr/share/capsem/profiles/"' in repack_deb
    )
    assert "sync-dev-assets.sh" not in repack_deb
    assert "capsem-admin" in repack_deb
    assert "capsem-tui" in repack_deb
    assert "/usr/share/capsem/assets" in deb_postinst
    assert "/usr/share/capsem/profiles" in deb_postinst
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest.json "$CAPSEM_DIR/assets/manifest.json"'
        not in deb_postinst
    )
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest-metadata.json "$CAPSEM_DIR/assets/manifest-metadata.json"'
        in deb_postinst
    )
    assert "event=manifest_copied" not in deb_postinst
    assert "manifest check" not in deb_postinst
    assert "event=manifest_report" not in deb_postinst
    assert "MANIFEST_METADATA=$(tr" in deb_postinst
    assert "event=manifest_metadata" in deb_postinst
    assert "MANIFEST_SOURCE=$(sed" in deb_postinst
    assert (
        'MANIFEST_SOURCE="https://release.capsem.org/assets/stable/manifest.json"'
        not in deb_postinst
    )
    assert "packaged manifest-metadata.json missing" in deb_postinst
    assert "packaged manifest-metadata.json has no manifest_url" in deb_postinst
    assert "event=manifest_source" in deb_postinst
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets --manifest \\"$MANIFEST_SOURCE\\"'
        in deb_postinst
    )
    assert "event=manifest_installed" in deb_postinst
    assert "event=assets_hydrated" not in deb_postinst
    assert "event=asset_hydration_failed" not in deb_postinst
    assert "event=assets_copied" not in deb_postinst
    assert 'echo "capsem: packaged binary missing: /usr/bin/$bin" >&2' in deb_postinst
    assert "event=binary_missing bin=$bin" in deb_postinst
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in deb_postinst
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in deb_postinst
    assert "install-current-run" in deb_postinst
    assert "install-latest.log" in deb_postinst
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in deb_postinst
    assert 'PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n' not in deb_postinst
    assert '[ "$READY_PROFILES" = "$TOTAL_PROFILES" ]' not in deb_postinst
    assert '[ "$TOTAL_PROFILES" -gt 0 ]' not in deb_postinst
    assert "event=profiles_not_ready" not in deb_postinst
    assert "capsem-admin" in deb_postinst
    assert "capsem-tui" in deb_postinst

    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest.json" "$CAPSEM_DIR/assets/manifest.json"'
        not in pkg_postinstall
    )
    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest-metadata.json" "$CAPSEM_DIR/assets/manifest-metadata.json"'
        in pkg_postinstall
    )
    assert "event=manifest_copied" not in pkg_postinstall
    assert "manifest check" not in pkg_postinstall
    assert "event=manifest_report" not in pkg_postinstall
    assert "MANIFEST_METADATA=$(tr" in pkg_postinstall
    assert "event=manifest_metadata" in pkg_postinstall
    assert "MANIFEST_SOURCE=$(sed" in pkg_postinstall
    assert (
        'MANIFEST_SOURCE="https://release.capsem.org/assets/stable/manifest.json"'
        not in pkg_postinstall
    )
    assert "packaged manifest-metadata.json missing" in pkg_postinstall
    assert "packaged manifest-metadata.json has no manifest_url" in pkg_postinstall
    assert "event=manifest_source" in pkg_postinstall
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets --manifest \\"$MANIFEST_SOURCE\\"'
        in pkg_postinstall
    )
    assert "event=manifest_installed" in pkg_postinstall
    assert "event=assets_hydrated" not in pkg_postinstall
    assert "event=asset_hydration_failed" not in pkg_postinstall
    assert "event=assets_copied" not in pkg_postinstall
    assert 'echo "capsem: packaged binary missing: $src" >&2' in pkg_postinstall
    assert "event=binary_missing bin=$bin" in pkg_postinstall
    assert 'source "$(dirname "$0")/install-user"' in pkg_postinstall
    assert "capsem_resolve_install_user" in pkg_postinstall
    assert "skipping per-user install" not in pkg_postinstall
    assert "secure install-user request" in pkg_install_user
    assert "/var/run/capsem/install-user" in pkg_install_user
    assert 'rm -rf "$CAPSEM_DIR"/bin.backup*' in pkg_postinstall
    assert "event=retired_binary_backups_removed" in pkg_postinstall


def test_macos_postinstall_adds_capsem_bin_to_fish_path() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()

    assert ".config/fish/config.fish" in postinstall
    assert "fish_add_path" in postinstall
    assert "grep -qF 'fish_add_path --path \"$HOME/.capsem/bin\"'" in postinstall
    assert 'cp -R "$PKG_SHARE/assets/"* "$CAPSEM_DIR/assets/"' not in postinstall
    assert "pkill -x capsem-app" in postinstall
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in postinstall
    assert 'INSTALL_RUN_ID=$(cat "$INSTALL_RUN_FILE" 2>/dev/null || date' in postinstall
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in postinstall
    assert "install-latest.log" in postinstall
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in postinstall
    assert "event=readiness_poll" in postinstall
    assert "attempt=$attempt" in postinstall
    assert 'PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n' not in postinstall
    assert '[ "$READY_PROFILES" = "$TOTAL_PROFILES" ]' not in postinstall
    assert '[ "$TOTAL_PROFILES" -gt 0 ]' not in postinstall
    assert "event=profiles_not_ready" not in postinstall


def test_linux_postinstall_prints_service_journal_on_readiness_failure() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()

    assert "event=service_diagnostics" in postinstall
    assert "systemctl --user status capsem.service --no-pager -l" in postinstall
    assert "journalctl --user-unit capsem.service --no-pager -n 100" in postinstall


def test_release_workflow_decouples_vm_assets_and_keeps_full_host_binary_set() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()

    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "run: just test-clean" not in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "kind: profiles" in workflow
    assert "output: target/binary-public-before/profiles" in workflow
    assert "output: target/candidate-profile-inputs" in workflow
    assert "--input-dir target/candidate-profile-inputs" in workflow
    assert "just qualify-binaries" in workflow
    assert "just qualify-binaries" in workflow
    assert "just _build-kernel" not in workflow
    assert "just _build-rootfs" not in workflow
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert (
        "ASSET_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json"
        in workflow
    )
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "-p capsem-admin" in workflow


def test_release_workflow_retries_app_cargo_tool_installs_through_config_authority() -> None:
    workflow_path = PROJECT_ROOT / ".github" / "workflows" / "release.yaml"
    workflow = yaml.safe_load(workflow_path.read_text())
    mac_steps = workflow["jobs"]["build-app-macos"]["steps"]
    linux_steps = workflow["jobs"]["build-app-linux"]["steps"]
    installer = next(
        step for step in mac_steps if step.get("name") == "Install exact config-owned Cargo tools"
    )
    config = tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text())
    configured = {tool["name"]: tool for tool in config["toolchain"]["crates"]}

    assert installer["run"].split() == [
        "uv",
        "run",
        "python",
        "scripts/install-configured-cargo-tools.py",
        "cargo-tauri",
        "cargo-sbom",
    ]
    assert str(installer["env"]["CARGO_NET_RETRY"]) == "10"
    assert "continue-on-error" not in installer
    for name in ("cargo-tauri", "cargo-sbom"):
        tool = configured[name]
        assert tool["install"][:2] == ["cargo", "install"]
        assert "--version" in tool["install"]
        assert tool["install"][-1] == "--locked"

    build_app_linux = "\n".join(str(step.get("run", "")) for step in linux_steps)
    assert "uv run capsem-gate cross-compile" in build_app_linux
    assert "install-configured-cargo-tools.py" not in build_app_linux
    assert "cargo install" not in build_app_linux
    assert "sudo apt-get" not in build_app_linux
    workflow_text = workflow_path.read_text()
    assert "-p capsem-tui" in workflow_text
    assert "-p capsem-mcp-aggregator" in workflow_text
    assert "-p capsem-mcp-builtin" in workflow_text
    assert "capsem-admin" in workflow_text
    assert "capsem-tui" in workflow_text
    assert "capsem-mcp-aggregator" in workflow_text
    assert "capsem-mcp-builtin" in workflow_text


def test_release_workflow_sets_up_uv_before_uv_run_steps() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    jobs_with_uv = {
        name: block for name, block in _workflow_job_blocks(workflow).items() if "uv run" in block
    }

    assert jobs_with_uv
    for name, block in jobs_with_uv.items():
        setup_pos = block.find("astral-sh/setup-uv@")
        uv_run_pos = block.find("uv run")
        assert setup_pos != -1, f"{name} uses uv run without setup-uv"
        assert setup_pos < uv_run_pos, f"{name} sets up uv after first uv run"


def test_ci_install_job_sets_up_uv_before_the_shared_install_gate() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    install_job = _workflow_job_blocks(workflow)["test-install"]

    setup_pos = install_job.find("astral-sh/setup-uv@")
    install_pos = install_job.find("uv run capsem-gate install")
    assert setup_pos != -1, "test-install invokes uv-backed Just helpers without setup-uv"
    assert setup_pos < install_pos, "test-install sets up uv after the shared install gate"


def test_ci_install_job_selects_exact_profiles_before_building_packages() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    install_job = _workflow_job_blocks(workflow)["test-install"]
    fetch_action = (
        PROJECT_ROOT / ".github" / "actions" / "fetch-release-inputs" / "action.yaml"
    ).read_text()

    fetch_pos = install_job.index("./.github/actions/fetch-release-inputs")
    resolve_pos = install_job.index("scripts/select-runtime-preflight-manifest.py")
    source_pos = install_job.index("scripts/fetch-channel-source-manifest.py")
    stage_pos = install_job.index("scripts/stage-release-test-inputs.py")
    materialize_pos = install_job.index("bash scripts/materialize-config.sh")
    package_pos = install_job.index("uv run capsem-gate cross-compile x86_64")
    gate_pos = install_job.index("uv run capsem-gate install")
    assert (
        resolve_pos < source_pos < fetch_pos < stage_pos < materialize_pos < package_pos < gate_pos
    )
    assert "bash scripts/materialize-config.sh --pair-content" in install_job
    assert "kind: profiles" in install_job
    assert "architecture: x86_64" in install_job
    assert "output: target/ci-install-content/inputs" in install_job
    assert "--input-dir target/ci-install-content/inputs" in install_job
    assert "Build exact native release package" in install_job
    assert install_job.count("steps.install-manifest.outputs.manifest-url") == 2
    assert "--classify-only" in install_job
    assert "published)" in install_job
    assert "retired)" in install_job
    assert "--require-profile-membership" in install_job
    assert (
        'output="$PWD/target/ci-install-selection/assets/stable/manifest.json"'
        in install_job
    ), "the retired first-party fixture must retain public stable channel identity"
    assert "file://$output" in install_job
    assert "CAPSEM_INSTALL_MANIFEST_URL: ${{ steps.install-manifest.outputs.manifest-url }}" in (
        install_job
    )
    assert "CAPSEM_INSTALL_CHANNEL: stable" in install_job
    assert "just _build-host-image" not in install_job
    assert "actions/cache/restore@" in fetch_action
    assert "actions/cache/save@" in fetch_action
    assert "--cache-dir target/release-input-cache" in fetch_action
    assert "--prune-cache" not in fetch_action
    assert "steps.fetch.outputs.cache-misses != '0'" in fetch_action
    assert "inputs.manifest-url" not in fetch_action.split("key:", 1)[1].splitlines()[0]
    assert "inputs.channel" not in fetch_action
    assert install_job.count("--content-root target/ci-install-content") == 1
    assert install_job.count("--selected-content-root target/ci-install-content") == 1
    assert "CAPSEM_INSTALL_PROFILE_INPUTS" not in install_job
    assert "scripts/prepare-install-test-assets.sh" not in install_job


def test_installed_doctor_failure_is_printed_and_preserved() -> None:
    probe = (PROJECT_ROOT / "scripts" / "release_installed_probe.py").read_text()

    assert 'doctor_log="$EVIDENCE_DIR/$label-doctor.log"' in probe
    assert 'failed_process_logs="$EVIDENCE_DIR/$label-failed-process-logs.txt"' in probe
    assert 'if ! CAPSEM_HOME="$CAPSEM_HOME_DIR" CAPSEM_RUN_DIR="$CAPSEM_HOME_DIR/run"' in probe
    assert 'find "$CAPSEM_HOME_DIR/run/sessions"' in probe
    assert 'cat "$doctor_log" >&2' in probe
    assert 'cat "$failed_process_logs" >&2' in probe


def test_ci_install_job_uploads_glowup_evidence_on_failure() -> None:
    workflow = yaml.safe_load((PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text())
    upload = next(
        step
        for step in workflow["jobs"]["test-install"]["steps"]
        if step.get("name") == "Upload install and glow-up evidence on failure"
    )

    assert upload["if"] == "failure()"
    assert upload["uses"] == "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
    assert set(upload["with"]["path"].splitlines()) == {
        "target/gate-runs/",
        "target/local-release-glowup-evidence/",
    }
    assert upload["with"]["if-no-files-found"] == (
        "${{ steps.install_e2e.outcome == 'failure' && 'error' || 'warn' }}"
    )


def test_asset_build_recipes_skip_kvm_only_for_build_prereq_doctor() -> None:
    """The asset build's doctor skips the two checks it is about to satisfy.

    An asset build cannot pass an asset check before it has built the assets,
    and it does not need KVM to build them. Every *other* doctor run keeps both.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    doctor_probe = subprocess.run(
        [
            "bash",
            "-c",
            """
set -eu
source scripts/doctor-linux.sh
section() { :; }
pass() { printf 'PASS:%s\\n' "$1"; }
fail() { printf 'FAIL:%s\\n' "$1"; }
warn() { printf 'WARN:%s\\n' "$1"; }
skip() { printf 'SKIP:%s\\n' "$1"; }
check_platform
""",
        ],
        cwd=PROJECT_ROOT,
        env={**os.environ, "CAPSEM_SKIP_KVM_CHECK": "1"},
        capture_output=True,
        text=True,
        check=True,
    )
    platform_lines = doctor_probe.stdout.splitlines()
    assert "SKIP:/dev/kvm (CAPSEM_SKIP_KVM_CHECK set)" in platform_lines
    assert "SKIP:/dev/vhost-vsock (CAPSEM_SKIP_KVM_CHECK set)" in platform_lines

    assert dict(config.imagebuild.doctor_skips) == {
        "CAPSEM_SKIP_ASSET_CHECK": "1",
        "CAPSEM_SKIP_KVM_CHECK": "1",
    }
    assert "CAPSEM_SKIP_KVM_CHECK" in _planned(
        "build-assets", profile="code", arch="arm64", template="all"
    )
    assert "CAPSEM_SKIP_KVM_CHECK" not in _planned("smoke")


def test_only_systemd_package_proof_receives_kvm_devices() -> None:
    """The build container gets no VM devices; only the proof that boots one does.

    A builder with `/dev/kvm` is a builder that can be made to do more than
    build, and nothing in a cross-compile needs it.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    issued = _planned("cross-compile", arch="arm64")
    container = (PROJECT_ROOT / "src/capsem/gate/installcontainer.py").read_text(encoding="utf-8")

    for device in config.install.vm_devices:
        assert device not in issued, f"the package builder is handed {device}"

    assert "--device" in container
    assert "vm_devices" in container


def test_cross_compile_clock_sync_uses_bounded_colima_command(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from helpers.gate import RecordingRunner

    from capsem.gate import config as gate_config
    from capsem.gate.content import ProfileContent
    from capsem.gate.packagerail import PackageRail

    config = gate_config.load(PROJECT_ROOT)
    target = config.arch("arm64")

    # Clock drift belongs to Colima's VM. Linux package builds keep the same
    # graph phase for stable timing/evidence, but the phase must issue no
    # command on a native Linux daemon.
    for system, machine, expected in (
        ("Linux", "x86_64", False),
        ("Darwin", "arm64", True),
    ):
        monkeypatch.setattr("capsem.gate.host.system", lambda system=system: system)
        monkeypatch.setattr("capsem.gate.host.machine", lambda machine=machine: machine)
        runner = RecordingRunner(PROJECT_ROOT)

        PackageRail(runner, target, content=ProfileContent.standalone(config)).sync_clock()

        assert runner.ran(re.escape(config.package.clock_script)) is expected

    # Prove the configured helper is actually bounded and reaches Colima
    # directly; a source-string check in the Just dispatcher went stale as
    # soon as the work moved into the package rail.
    clock_path = PROJECT_ROOT / config.package.clock_script
    spec = importlib.util.spec_from_file_location("install_payload_clock_sync", clock_path)
    assert spec is not None and spec.loader is not None
    clock = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(clock)
    calls: list[tuple[list[str], dict[str, object]]] = []

    def record(command, **kwargs):
        calls.append((command, kwargs))
        return subprocess.CompletedProcess(command, 0, stdout="synced\n", stderr="")

    monkeypatch.setattr(clock.subprocess, "run", record)
    clock.sync_container_clock(timeout_seconds=7)

    command, options = calls[0]
    assert command[:3] == ["colima", "ssh", "--"]
    assert "docker" not in command
    assert options["timeout"] == 7
    assert options["check"] is True


def test_security_event_rows_go_through_security_engine_emitter() -> None:
    roots = [
        PROJECT_ROOT / "crates" / "capsem-core" / "src",
        PROJECT_ROOT / "crates" / "capsem-process" / "src",
    ]
    allowed_files = {
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "security_engine" / "mod.rs",
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "security_engine" / "tests.rs",
    }
    patterns = [
        "write(WriteOp::",
        "write(capsem_logger::WriteOp::",
        "try_write(WriteOp::",
        "try_write(capsem_logger::WriteOp::",
        "try_emit_security_write(",
    ]

    violations: list[str] = []
    for root in roots:
        for path in root.rglob("*.rs"):
            if path in allowed_files or "/tests/" in path.as_posix():
                continue
            text = path.read_text()
            for lineno, line in enumerate(text.splitlines(), start=1):
                if any(pattern in line for pattern in patterns):
                    rel = path.relative_to(PROJECT_ROOT)
                    violations.append(f"{rel}:{lineno}: {line.strip()}")

    assert not violations, (
        "security/logging rows must be emitted through "
        "capsem_core::security_engine::{emit_security_write,emit_security_write_blocking}; "
        "direct DbWriter WriteOp sends found:\n" + "\n".join(violations)
    )
