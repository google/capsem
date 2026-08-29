"""Release doctor contract tests."""

from __future__ import annotations

import functools
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from types import SimpleNamespace
from typing import ClassVar

import pytest
import yaml
from capsem_builder.gate.shellnodes import arm_named
from capsem_builder.gate.shellparse import parse as parse_shell
from capsem_builder.gate.tools.audit import release_selections
from capsem_builder.gate.tools.web import check_cloudflare_pages_project as CLOUDFLARE_PROJECT
from capsem_builder.release.tools import build_complete_release_channel as COMPLETE_CHANNEL
from capsem_builder.release.tools import check_remote_release_readiness as READINESS
from capsem_builder.release.tools import local_release_glowup as LOCAL_GLOWUP
from capsem_builder.release.tools import remote_ci_gate as REMOTE_CI_GATE
from capsem_builder.release.tools import verify_channel_downloads as VERIFY_DOWNLOADS
from capsem_builder.release.tools import (
    write_binary_channel_staging_proof as BINARY_STAGING_PROOF,
)
from capsem_builder.release.tools import write_release_summary as RELEASE_SUMMARY
from helpers.workflow_contract import assert_unmasked_step, parsed_commands, workflow_reachable_text

PROJECT_ROOT = Path(__file__).resolve().parents[3]

# Every job `pr-gate` must aggregate before a pull request can merge. Declared
# once, as a set: this is the contract, so it is stated here rather than read
# back from the workflow it judges. It was previously spelled as the exact
# string `needs: [fast-gate, test-linux, ...]` in six places, which made
# reordering the list -- or writing it in YAML block style, which is the same
# list -- fail four contracts while changing nothing GitHub acts on.
REQUIRED_PR_GATE_JOBS = frozenset(
    {
        "scope",
        "fast-gate",
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
    }
)

# The Rust line-coverage floor `just test-clean` enforces. Named once because two
# separate contracts assert it, and a floor that disagrees with itself is worse
# than no floor. It tracks the real measured surface: adding previously
# unmonitored crates to the measurement lowered the percentage without
# removing a single test, and the floor followed the measurement rather than
# tests being written to flatter it.
RUST_LINE_COVERAGE_FLOOR = "--fail-under-lines 63"
FAST_DOCTOR_FLAG = "doctor " + "--" + "fast"
OLD_DEBUG_CRATE = "capsem-debug" + "-upstream"


def _rootfs_obom_bytes(architecture: str = "arm64") -> bytes:
    return (
        json.dumps(
            {
                "bomFormat": "CycloneDX",
                "metadata": {
                    "component": {
                        "type": "operating-system",
                        "name": f"capsem-rootfs-{architecture}",
                        "version": "guest-rootfs",
                        "properties": [
                            {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                            {"name": "capsem:guest:architecture", "value": architecture},
                        ],
                    },
                    "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                },
                "components": [
                    {
                        "type": "library",
                        "name": "apt",
                        "version": "2.6.1",
                        "purl": "pkg:deb/debian/apt@2.6.1?distro=debian-12",
                    }
                ],
            },
            sort_keys=True,
        )
        + "\n"
    ).encode()


def _readiness_checker_module():
    return importlib.reload(READINESS)


def _release_site_contract_module():
    module_path = PROJECT_ROOT / "scripts/check-release-site-contract.py"
    spec = importlib.util.spec_from_file_location("check_release_site_contract", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _cloudflare_pages_project_module():
    return importlib.reload(CLOUDFLARE_PROJECT)


def _boot_timing_module():
    module_path = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "boot_timing.py"
    spec = importlib.util.spec_from_file_location("capsem_doctor_boot_timing", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _doctor_runtimes_module():
    module_path = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_runtimes.py"
    spec = importlib.util.spec_from_file_location("capsem_doctor_runtimes", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    host_conftest = sys.modules.get("conftest")
    sys.modules["conftest"] = SimpleNamespace(
        run=lambda *_args, **_kwargs: SimpleNamespace(returncode=0, stdout="", stderr="")
    )
    try:
        spec.loader.exec_module(module)
    finally:
        if host_conftest is None:
            del sys.modules["conftest"]
        else:
            sys.modules["conftest"] = host_conftest
    return module


#: Recipes whose behaviour moved into the gate, and the command that owns it
#: now. These contracts are about what the gate *does*; when the doing moved
#: from a shell body into a plan, the place to read it moved with it.
_DISPATCHED = {
    "_test-candidate:": ("test-candidate", {}),
    "_test-fast:": ("test-fast", {}),
    "_gate-assets:": ("assets", {}),
    "_gate-install:": ("install", {}),
    "_gate-linux-rust:": ("linux-rust", {}),
    "_gate-host-package-sbom:": ("host-sbom", {}),
    "_cross-compile": ("cross-compile", {"arch": "arm64"}),
    "release-binaries": ("release-binaries", {"channel": "nightly"}),
    "release-profile": ("release-profile", {"channel": "nightly", "profile": "code"}),
    "_build-assets": ("build-assets", {"profile": "code", "arch": "arm64", "template": "all"}),
    "_pack-initrd:": ("pack-initrd", {}),
    "_docker-gc:": ("storage", {"action": "gc", "rail": None}),
}


def _issued(command: str, args: tuple) -> str:
    """Every command a gate command would actually run. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_issued

    return gate_issued(command, args)


#: `test-clean:` is the whole local diagnostic, and running its plan against a
#: recording runner
#: stops at the first step that needs a real machine. So the text for it is the
#: union of what each phase issues, gathered by running each module command --
#: which is the same work, reached without one failure hiding the rest.
def _whole_gate() -> tuple[tuple[str, dict], ...]:
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import WHOLE_GATE

    return WHOLE_GATE


@functools.cache
def _dispatched_text(name: str) -> str:
    if name in {"test-clean:", "_test-candidate:"}:
        return "\n".join(
            _issued(command, tuple(sorted(args.items()))) for command, args in _whole_gate()
        )
    command, args = _DISPATCHED[name]
    return _issued(command, tuple(sorted(args.items())))


#: Each documented CI stage, and a step label that must exist in the composed
#: gate for that stage to be real. The names were section headers in a shell
#: body; they are phases now, and the documentation still compares the PR gate
#: against them -- so what is checked is that each documented name still
#: corresponds to work the gate actually does.
_GATE_STAGES = {
    "Audits + lint + web surfaces": "fast.audit.",
    "Cross-compile agent (both arches)": "static.guest-agents",
    "Rust: test suite with coverage": "static.rust-coverage",
    "Python: non-serial tests (n=4 parallel)": "functional.pytest.broad.",
    "Python: serial timing and benchmark tests": "functional.pytest.timing.",
    "Fast source and serialized release contracts": "contracts.release",
    "Injection test": "functional.injection.",
    "Integration test": "functional.integration.",
    "Benchmarks": "functional.pytest.benchmark.",
    "Cross-compile Linux releases (Docker, both arches)": "package.",
    "Install e2e tests (Docker + systemd)": "glowup.install",
}


def _gate_labels() -> tuple[str, ...]:
    """Every step of the complete gate. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return gate_labels()


def _recipe_block(name: str) -> str:
    """The recipe, and the plan it dispatches to.

    Both, because these contracts predate the extraction and each is about the
    behaviour rather than where it is written. A recipe is a dispatch now, so
    reading only its body answers a question nobody is asking.
    """
    block = _recipe_body(name)
    if name in _DISPATCHED:
        block = block + "\n" + _dispatched_text(name)
    return block


def _recipe_body(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    stem = name.removesuffix(":")
    start = next(
        i
        for i, line in enumerate(lines)
        if line == name
        or line.startswith(f"{name} ")
        or (line.startswith(f"{stem} ") and line.endswith(":"))
    )
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_path(name: str) -> Path:
    return PROJECT_ROOT / ".github" / "workflows" / name


def _workflow_document(workflow_name: str = "ci.yaml") -> dict:
    """The parsed workflow. Every structural question is answered from here."""
    return yaml.safe_load(_workflow_path(workflow_name).read_text())


def _workflow_job(name: str, workflow_name: str = "ci.yaml") -> dict:
    """One job, as the mapping GitHub will actually act on."""
    jobs = _workflow_document(workflow_name).get("jobs") or {}
    assert name in jobs, f"{workflow_name} declares no job {name!r}"
    return jobs[name]


def _workflow_job_block(name: str, workflow_name: str = "ci.yaml") -> str:
    """The job's own source lines, located through the parsed document.

    Still text, because most assertions below are about the contents of `run:`
    scripts and a script body is text however it is reached. Located by YAML
    node marks rather than by matching an exact two-space-indented line: the
    old slice lost the job entirely after a reindent, and any comment at that
    indentation ending in `:` truncated the block early -- silently dropping
    every step below it from the assertions that follow.

    Structural properties no substring can see -- `continue-on-error`, `if:`,
    the membership of `needs` -- belong in `_workflow_job` and in
    `tests/citadel/test_workflow_enforcement.py`, not here.
    """
    _workflow_job(name, workflow_name)
    text = _workflow_path(workflow_name).read_text()
    root = yaml.compose(text)
    jobs = next(value for key, value in root.value if key.value == "jobs")
    key, node = next(pair for pair in jobs.value if pair[0].value == name)
    return "\n".join(text.splitlines()[key.start_mark.line : node.end_mark.line])


def _workflow_text(name: str) -> str:
    return _workflow_path(name).read_text()


def _source_text(path: str) -> str:
    package_sources = {
        "scripts/build-complete-release-channel.py": Path(COMPLETE_CHANNEL.__file__),
        "scripts/check-cloudflare-pages-project.py": Path(CLOUDFLARE_PROJECT.__file__),
        "scripts/check-remote-release-readiness.py": Path(READINESS.__file__),
        "scripts/local-release-glowup.py": Path(LOCAL_GLOWUP.__file__),
        "scripts/verify-channel-downloads.py": Path(VERIFY_DOWNLOADS.__file__),
        "scripts/write-binary-channel-staging-proof.py": Path(BINARY_STAGING_PROOF.__file__),
        "scripts/write-release-summary.py": Path(RELEASE_SUMMARY.__file__),
    }
    if source := package_sources.get(path):
        return source.read_text()
    return (PROJECT_ROOT / path).read_text()


def _skill_text(path: str) -> str:
    """Read a skill and the reference files it explicitly tells agents to load."""
    skill_path = PROJECT_ROOT / path
    skill_dir = skill_path.parent
    main = skill_path.read_text()
    parts = [main]
    for relative in dict.fromkeys(re.findall(r"`(references/[A-Za-z0-9_./-]+\.md)`", main)):
        reference = (skill_dir / relative).resolve()
        assert reference.is_relative_to(skill_dir.resolve())
        assert reference.is_file(), f"missing linked skill reference: {relative}"
        parts.append(reference.read_text())
    return "\n".join(parts)


def _command_attribute_prefix(source: str, struct_name: str = "Args") -> str:
    marker = f"struct {struct_name}"
    assert marker in source
    return source[: source.index(marker)]


def test_doctor_fix_builds_assets_for_each_checked_in_profile() -> None:
    source = (PROJECT_ROOT / "scripts" / "doctor-common.sh").read_text()

    assert "for profile in config/profiles/*/profile.toml" in source
    assert 'just _build-assets "$(basename "$(dirname "$profile")")" "$arch"' in source
    assert '"touch .dev-setup && CAPSEM_SKIP_ASSET_CHECK=1 just _build-assets"' not in source


def test_macos_doctor_requires_live_rosetta_registration() -> None:
    from capsem_builder.gate import config as gate_config

    source = _source_text("scripts/doctor-macos.sh")
    config = gate_config.load(PROJECT_ROOT)

    assert config.install.rosetta_binfmt in source
    assert "colima rosetta configured but not registered" in source
    assert "colima restart" in source

    # The asset gate proves Docker can execute the architecture this host is
    # *not*, before any lane starts -- discovering otherwise an hour in wastes
    # the whole matrix. Both platform names are derived from the architecture
    # table rather than spelled here.
    # The cross-execution probe moved to `crossexec`, which is its own
    # question: whether this daemon can run a foreign architecture is a
    # property of the machine, not of building assets.
    crossexec = _source_text("build_system/builder/gate/crossexec.py")
    assert '"--platform",' in crossexec
    # The probe goes through the Docker wrapper, which means it also has to
    # say what network it needs -- the property the migration was for, and a
    # stronger claim than the argv fragment this line used to match.
    assert "cross_platform_probe_network" in crossexec
    assert "build_arch.docker_platform" in crossexec
    assert "build_arch.base_image" in crossexec
    assert "cross_platform_probe_image" not in crossexec
    assert "Docker cannot execute {platform} containers" in crossexec
    assert "colima restart" in crossexec, "macOS needs its own remedy in the message"


def test_bootstrap_and_doctor_prove_tart_cache_clone_boot_and_ssh() -> None:
    bootstrap = _source_text("bootstrap.sh")
    doctor = _source_text("scripts/doctor-macos.sh")
    readiness = _source_text(
        "build_system/builder/image/tools/build/tart_readiness.py"
    )

    assert 'uv run --project build_system --frozen python "$SCRIPT_DIR/scripts/tart_readiness.py"' in bootstrap
    assert "--require-cache" in doctor
    assert "cached, cloned, booted, and SSH-ready" in doctor
    assert '"tart", "clone"' in readiness
    assert '"tart",\n                "run",' in readiness
    assert "wait_for_guest_ip" in readiness
    assert "wait_for_ssh" in readiness
    assert "cleanup_vm(vm_name)" in readiness


def test_host_sbom_zstd_dependency_has_local_and_binary_lane_parity() -> None:
    """The canonical gate must provision the same Debian archive decoder everywhere."""
    bootstrap = _source_text("bootstrap.sh")
    doctor = _source_text("scripts/doctor-common.sh")
    macos_doctor = _source_text("scripts/doctor-macos.sh")
    linux_doctor = _source_text("scripts/doctor-linux.sh")
    release = _workflow_text("release.yaml")

    assert 'confirm "zstd (Debian package/SBOM archive support, via brew)"' in bootstrap
    assert "brew install zstd" in bootstrap
    assert "for tool in cargo rustup node python3 uv pnpm sqlite3 git b3sum zstd" in doctor
    assert "zstd)" in macos_doctor
    assert 'echo "brew install zstd"' in macos_doctor
    assert "zstd)" in linux_doctor

    install = release.index("Install host SBOM archive deps")
    generate = release.index("Generate packaged host SBOM")
    assert "sudo apt-get install -y --no-install-recommends zstd" in release[install:generate]


def test_profile_release_builds_both_published_architectures() -> None:
    build_assets = _workflow_job_block("build-assets", "release-assets.yaml")
    assert "- arch: arm64" in build_assets
    assert "- arch: x86_64" in build_assets
    assert 'just build-assets ${{ matrix.arch }} "${{ inputs.profile }}"' in build_assets
    assert 'just build-assets ${{ matrix.arch }} "${{ inputs.profile }}"' in build_assets


def test_parallel_asset_gate_preserves_and_names_failed_architecture_logs() -> None:
    """Each lane logs separately, and a failing lane surfaces its own tail.

    Read out of the recipe when this was shell, where the lane's exit status
    came back through `wait` into a variable -- and a variable that goes unread
    turns a failed build into a passing gate. The behaviour is now asserted
    against the commands the lanes issue, in build_system/tests/gate/test_gate_assetlanes.py;
    what stays here is that both halves still exist.
    """
    from capsem_builder.gate import config as gate_config

    lanes = _source_text("build_system/builder/gate/assetlanes.py")
    config = gate_config.load(PROJECT_ROOT)

    # One log per lane, named for its architecture.
    assert 'f"build-{arch.name}.log"' in lanes
    # Both lanes are awaited before either result is read: cancelling the
    # second would leave its containers running and report one error for a run
    # that had two.
    # Awaiting both lanes is the scheduler's guarantee now, not a pool's:
    # two steps with no edge between them both run, and a failure skips only
    # what depends on it. `test_gate_assetlanes` proves it through a plan.
    assert "def build(self, arch" in lanes
    # The lane reports its own tail and re-raises; collecting both failures
    # is the plan's job, and doing it here needed a pool the graph could not
    # see.
    assert "self._report(arch, error)" in lanes
    assert "failure_tail_lines" in lanes
    assert config.assets.failure_tail_lines > 0


def test_asset_gate_interrupt_cleanup_only_reaps_owned_mounts(tmp_path: Path) -> None:
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    assets = _source_text("build_system/builder/gate/assets.py")

    # Scoped to this gate's own scratch root, and run from a `finally` so an
    # aborted lane still releases its containers.
    assert "container_cleanup_script" in assets
    assert "str(self.test_root)" in assets
    assert config.assets.container_cleanup_script.endswith("cleanup-docker-containers-by-mount.sh")

    mount_root = tmp_path / "asset-root"
    mount_root.mkdir()
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    removals = tmp_path / "removals.log"
    docker = fake_bin / "docker"
    docker.write_text(
        """#!/bin/sh
set -eu
if [ "$1" = "ps" ]; then
    printf 'owned\\nforeign\\n'
elif [ "$1" = "inspect" ]; then
    id="${4}"
    if [ "$id" = "owned" ]; then
        printf '%s/lane/arm64\\n' "$FAKE_MOUNT_ROOT"
    else
        printf '/tmp/unrelated\\n'
    fi
elif [ "$1" = "rm" ]; then
    printf '%s\\n' "${3}" >> "$FAKE_REMOVALS"
else
    exit 97
fi
"""
    )
    docker.chmod(0o755)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{fake_bin}:{env['PATH']}",
            "FAKE_MOUNT_ROOT": str(mount_root),
            "FAKE_REMOVALS": str(removals),
        }
    )

    result = subprocess.run(
        [
            "bash",
            str(PROJECT_ROOT / "scripts/cleanup-docker-containers-by-mount.sh"),
            str(mount_root),
        ],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert removals.read_text().splitlines() == ["owned"]


def test_canonical_gate_builds_both_linux_release_architectures() -> None:
    canonical_gate = _dispatched_text("test-clean:")

    arm64 = canonical_gate.index("package.arm64")
    x86_64 = canonical_gate.index("package.x86_64")
    install = canonical_gate.rindex("glowup.install")
    # Both architectures, in order, and both before the install proof that
    # consumes them. Never one unnamed architecture: the cohort is both or it
    # is not a cohort.
    assert arm64 < x86_64 < install


def test_install_e2e_reuses_exact_package_and_materialized_profile_config() -> None:
    """The install gate consumes the package rail's output; it builds nothing.

    This used to read the recipe text and assert `install_pos < stage_pos` --
    the install running before its assets were staged. That was not a contract
    but a transcription of the defect, and it would have failed the fix. The
    order now lives in `capsem_builder.gate.install` and is asserted against the
    commands the gate actually issues, in
    `build_system/tests/gate/test_gate_install_ordering.py`.
    """
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "install.py").read_text()
    proof = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "installproof.py").read_text()
    recipe = _recipe_block("_gate-install:")

    assert "capsem-gate install" in recipe

    # Consumes, never rebuilds.
    for builder in (
        "prepare-install-test-assets.sh",
        "materialize-config.sh",
        "repack-deb.sh",
        "_materialize-config",
    ):
        assert builder not in source + proof, f"the install gate must not run {builder}"

    # One typed, prevalidated assets/config pair reaches the container. Raw
    # manifest inputs are staged on the host and never transformed mid-proof.
    assert config.install.generated_inputs == (config.outputs.packages,)
    assert "stage_content" in proof
    assert "stage_inputs_script" not in proof
    assert "stage-release-test-inputs" not in proof
    assert 'cp -R "{assets}/." "{self._layout.assets}/"' in proof
    assert 'cp -R "{content_config}/." "{self._layout.config}/"' in proof
    assert "missing exact release-mode Debian package" in source
    assert "just _cross-compile" in source


def test_ci_materializes_runtime_profiles_after_generating_settings() -> None:
    workflow = _workflow_job_block("test")

    generate_pos = workflow.find("bash scripts/generate-settings.sh")
    prepare_assets_pos = workflow.find("bash scripts/prepare-install-test-assets.sh")
    materialize_pos = workflow.find("bash scripts/materialize-config.sh")
    python_pos = workflow.find("Python schema tests with coverage")

    assert generate_pos != -1
    assert prepare_assets_pos != -1
    assert materialize_pos != -1
    assert python_pos != -1
    assert generate_pos < prepare_assets_pos < materialize_pos < python_pos


def test_ci_python_schema_pytest_paths_exist() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    coverage_step = workflow.split(
        "- name: Cross-system Python schema tests with coverage", maxsplit=1
    )[1].split("# Python integration tests", maxsplit=1)[0]
    paths = sorted(
        set(re.findall(r"(?:build_system/)?tests/[^\s\\]+", coverage_step))
    )

    missing = [path for path in paths if not (PROJECT_ROOT / path).exists()]

    assert missing == []


def test_ci_has_stable_pr_gate_over_all_required_jobs() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    trigger = workflow.split("permissions:", maxsplit=1)[0]
    gate = workflow_reachable_text(
        PROJECT_ROOT, PROJECT_ROOT / ".github" / "workflows" / "ci.yaml", job="pr-gate"
    )
    release_site_job = _workflow_job_block("release-site-build")

    assert "pull_request:" in workflow
    assert "push:" in trigger
    assert "branches: [main]" in trigger
    assert "cancel-in-progress: ${{ github.event_name == 'pull_request' }}" in trigger
    assert frozenset(_workflow_job("pr-gate")["needs"]) == REQUIRED_PR_GATE_JOBS
    assert _workflow_job("pr-gate").get("if") == "${{ always() }}"
    assert "SCOPE_RESULT: ${{ needs.scope.result }}" in gate
    assert "FAST_GATE_RESULT: ${{ needs.fast-gate.result }}" in gate
    assert "TEST_LINUX_RESULT: ${{ needs.test-linux.result }}" in gate
    assert "TEST_MACOS_RESULT: ${{ needs.test.result }}" in gate
    assert "TEST_INSTALL_RESULT: ${{ needs.test-install.result }}" in gate
    assert "DOCS_BUILD_RESULT: ${{ needs.docs-build.result }}" in gate
    assert "SITE_BUILD_RESULT: ${{ needs.site-build.result }}" in gate
    assert "RELEASE_SITE_BUILD_RESULT: ${{ needs.release-site-build.result }}" in gate
    assert 'test "$FAST_GATE_RESULT" = success' in gate
    assert 'test "$TEST_LINUX_RESULT" = success' in gate
    assert 'test "$TEST_MACOS_RESULT" = success' in gate
    assert 'test "$TEST_INSTALL_RESULT" = success' in gate
    assert 'test "$DOCS_BUILD_RESULT" = success' in gate
    assert 'test "$SITE_BUILD_RESULT" = success' in gate
    assert 'test "$RELEASE_SITE_BUILD_RESULT" = success' in gate
    assert "astral-sh/setup-uv@" in release_site_job
    assert "uv sync --project build_system --frozen" in release_site_job
    assert "bash scripts/check-web-surface.sh release-site" in release_site_job


def test_pr_gate_blocks_broken_docs_and_marketing_builds() -> None:
    workflow = _workflow_text("ci.yaml")
    docs_job = _workflow_job_block("docs-build")
    site_job = _workflow_job_block("site-build")
    gate = workflow_reachable_text(
        PROJECT_ROOT, PROJECT_ROOT / ".github" / "workflows" / "ci.yaml", job="pr-gate"
    )
    docs_deploy = _workflow_text("docs.yaml")
    site_deploy = _workflow_text("site.yaml")
    docs_ci = _source_text("docs/src/content/docs/development/ci.md")
    docs_ci_text = " ".join(docs_ci.split())

    assert "pr-gate:" in workflow
    assert "docs-build:" in workflow
    assert "site-build:" in workflow
    assert frozenset(_workflow_job("pr-gate")["needs"]) == REQUIRED_PR_GATE_JOBS
    assert "DOCS_BUILD_RESULT: ${{ needs.docs-build.result }}" in gate
    assert "SITE_BUILD_RESULT: ${{ needs.site-build.result }}" in gate
    assert "RELEASE_SITE_BUILD_RESULT: ${{ needs.release-site-build.result }}" in gate
    assert 'test "$DOCS_BUILD_RESULT" = success' in gate
    assert 'test "$SITE_BUILD_RESULT" = success' in gate
    assert 'test "$RELEASE_SITE_BUILD_RESULT" = success' in gate

    assert "cache-dependency-path: docs/pnpm-lock.yaml" in docs_job
    assert "cd docs && pnpm install --frozen-lockfile" in docs_job
    assert "bash scripts/check-web-surface.sh docs" in docs_job
    assert "pages deploy" not in docs_job

    assert "cache-dependency-path: site/pnpm-lock.yaml" in site_job
    assert "cd site && pnpm install --frozen-lockfile" in site_job
    assert "bash scripts/check-web-surface.sh site" in site_job
    assert "pages deploy" not in site_job

    assert "pull_request:" not in docs_deploy
    assert "pull_request:" not in site_deploy
    assert "push:" in docs_deploy
    assert "push:" in site_deploy
    assert "branches: [main]" in docs_deploy
    assert "branches: [main]" in site_deploy

    assert "docs-build" in docs_ci
    assert "site-build" in docs_ci
    assert (
        "`pr-gate` depends on `docs-build`, `site-build`, and `release-site-build`" in docs_ci_text
    )


def test_macos_ci_installs_release_site_dependencies_before_integration() -> None:
    job = _workflow_job_block("test")
    install = "cd build_system/release_site && pnpm install --frozen-lockfile"
    integration = "Python integration tests (non-VM suites)"

    assert "web/app/pnpm-lock.yaml" in job
    assert "build_system/release_site/pnpm-lock.yaml" in job
    assert install in job
    assert job.index(install) < job.index(integration)


def test_ci_test_steps_do_not_mask_failures_with_true() -> None:
    workflow = yaml.safe_load(
        (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text(encoding="utf-8")
    )

    for job_name, step_name in (
        ("test-linux", "Unit tests (KVM backend) with coverage"),
        ("test", "Unit tests with coverage"),
        ("test", "Integration tests with coverage"),
        ("test", "Build frontend bundle"),
        ("test", "Frontend type-check and test"),
        ("test", "Cross-system Python schema tests with coverage"),
        ("test", "Build-system Python schema tests with coverage"),
        ("test", "Python integration tests (non-VM suites)"),
        ("test", "Verify all integration test imports"),
        ("test", "Schema drift check"),
        ("test-install", "Run install e2e tests"),
        ("test-install", "Stage one exact install content pair"),
        ("docs-build", "Build docs"),
        ("site-build", "Build site"),
    ):
        assert_unmasked_step("ci.yaml", workflow, job_name, step_name)


def test_release_channel_contract_suite_is_in_pr_and_local_gates() -> None:
    workflow = _workflow_job_block("test")
    just_test = _dispatched_text("test-clean:")
    local_suite = _source_text("tests/capsem-release/test_release_channel_contract.py")

    assert "tests/capsem-release/" in workflow
    assert "Python integration tests (non-VM suites)" in workflow
    assert "tests/capsem-release/" in just_test
    # The broad run ignores it on purpose: it is the release-contracts phase
    # that owns this suite, and running it twice in one gate would double a
    # four-minute cost to prove the same thing. Read from the configuration
    # that declares the ignore rather than from a recipe comment.
    from capsem_builder.gate import config as gate_config

    settings = gate_config.load(PROJECT_ROOT).suites.pytest
    assert "tests/capsem-release" in settings.broad_ignores
    assert "contracts.release" in just_test
    assert "validator.validate_release_site(" in local_suite
    assert "test_release_channel_contract_rejects_swapped_manifest" in local_suite
    assert "test_release_channel_contract_ignores_stale_health_summary" in local_suite
    assert "test_release_channel_contract_rejects_cache_header_drift" in local_suite
    assert "test_two_generated_release_channels_have_same_machine_contract" in local_suite


def test_release_workflows_run_disjoint_lane_policy_gates() -> None:
    binary_workflow = _workflow_text("release.yaml")
    asset_workflow = _workflow_text("release-assets.yaml")
    deploy_workflow = _workflow_text("release-channel.yaml")
    ci_workflow = _workflow_job_block("test")

    binary_trigger = binary_workflow.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" in binary_trigger
    assert "push:" not in binary_trigger
    assert "tag:" in binary_trigger
    assert "channel:" in binary_trigger
    assert "Verify binary release lane policy" in binary_workflow
    assert "tests/capsem-release/test_binary_lane_gate.py" in binary_workflow
    assert "tests/capsem-release/test_release_lane_diff_policy.py" in binary_workflow
    assert "just _build-kernel" not in binary_workflow
    assert "just _build-rootfs" not in binary_workflow

    assert "workflow_dispatch:" in asset_workflow
    assert "profile:" in asset_workflow
    assert "Verify profile release lane policy" in asset_workflow
    assert "tests/capsem-release/test_profile_lane_gate.py" in asset_workflow
    assert "tests/capsem-release/test_release_lane_diff_policy.py" in asset_workflow
    assert "BINARY_VERSION" not in asset_workflow
    assert "Record binary release metadata" not in asset_workflow

    assert "workflow_call:" in deploy_workflow
    assert "cargo run -p capsem-admin -- assets channel build" not in deploy_workflow
    assert "cloudflare/wrangler-action@" in deploy_workflow

    assert "tests/capsem-release/" in ci_workflow


def test_install_e2e_generates_manifest_through_admin_rail() -> None:
    script = (PROJECT_ROOT / "scripts" / "prepare-install-test-assets.sh").read_text()

    assert "cargo run -p capsem-admin -- manifest generate" in script
    assert "arm64|aarch64)" in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/vmlinuz"' in script
    assert 'create_minimal_initrd_if_missing "$ASSETS_DIR/$arch/initrd.img"' in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/initrd.img"' not in script
    assert "cpio -o -H newc" not in script
    assert "gzip.GzipFile" in script
    assert "mtime=0" in script
    assert "TRAILER!!!" in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/rootfs.erofs"' in script
    assert "scripts/gen_manifest.py" not in script


def test_profile_release_builds_one_profile_against_resolved_binary() -> None:
    # The workflow plus every script it dispatches to. A step that grew past
    # the shell-body ceiling and moved into `scripts/` runs the same commands;
    # asserting against the workflow text alone made that refactor look like a
    # regression, which is how a literal contract punishes the right change.
    workflow = workflow_reachable_text(PROJECT_ROOT, _workflow_path("release-assets.yaml"))
    fast_gate = _workflow_text("fast-gate.yaml")
    trigger = workflow.split("\npermissions:", maxsplit=1)[0]

    assert "workflow_dispatch:" in workflow
    assert "channel:" in trigger
    assert "profile:" in trigger
    assert "default: stable" not in trigger
    assert "default: code" not in trigger
    assert "push:" not in workflow
    assert "tags:" not in workflow
    assert "group: capsem-release-${{ inputs.channel }}" in workflow
    assert "cancel-in-progress: false" in workflow
    assert "deployments: write" in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "kind: packages" in workflow
    assert "output: target/profile-public-before/packages" in workflow
    assert "--input-dir target/profile-public-before/packages" in workflow
    assert "--print-package-path" in workflow
    assert 'just build-assets ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert 'just build-assets ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert "- arch: arm64" in workflow
    assert "- arch: x86_64" in workflow
    assert "cargo run -p capsem-admin -- release" in workflow
    assert "--publication-base" in workflow
    assert "stage-profile-publication.py" in workflow
    assert "verify-profile-publication.py" in workflow
    assert "build_system/packaging/macos/build-pkg.sh" not in workflow
    assert "build_system/packaging/linux/repack-deb.sh" not in workflow
    assert "cargo tauri build" not in workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate
    assert "just qualify-assets" in workflow
    assert "just _test-release-contracts" not in workflow
    assert "scripts/build-complete-release-channel.py" in workflow
    assert "channel-source-$CHANNEL.json" in workflow
    assert "check-profile-release-delta.py" in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: asset-channel-preview" in workflow
    assert (
        "if: ${{ inputs.dry_run == false && needs.publish-profile-release.outputs.release_needed == 'true' && needs.publish-profile-release.outputs.activation_ready == 'true' }}"
        in workflow
    )


def test_asset_channel_deploy_consumes_generated_dist_artifact() -> None:
    workflow = _workflow_text("release-channel.yaml")

    assert "workflow_call:" in workflow
    assert "workflow_dispatch:" not in workflow
    assert "dist_artifact:" in workflow
    assert "deploy_branch:" in workflow
    assert "release_site_url:" in workflow
    assert "default: main" in workflow
    assert "default: https://release.capsem.org" in workflow
    assert "secrets:" in workflow
    assert "CLOUDFLARE_ACCOUNT_ID:" in workflow
    assert "CLOUDFLARE_API_TOKEN:" in workflow
    assert "required: true" in workflow
    assert "actions/download-artifact@" in workflow
    assert "DIST_DIR: target/distribution" in workflow
    assert 'test -f "$DIST_DIR/index.html"' in workflow
    assert 'test -f "$DIST_DIR/health.json"' in workflow
    assert 'test -f "$DIST_DIR/_headers"' in workflow
    assert 'test -f "$DIST_DIR/assets/$CHANNEL/manifest.json"' in workflow
    assert 'find "$DIST_DIR" -type f -size +25M' in workflow
    assert "Pages dist contains oversized file" in workflow
    assert "cargo run -p capsem-admin -- assets channel build" not in workflow
    assert "Require Cloudflare credentials" in workflow
    assert "CLOUDFLARE_ACCOUNT_ID secret is required to deploy release.capsem.org" in workflow
    assert "CLOUDFLARE_API_TOKEN secret is required to deploy release.capsem.org" in workflow
    assert "Verify Cloudflare Pages project" in workflow
    assert "RELEASE_CHANNEL_PROJECT: release" in workflow
    assert "python scripts/check-cloudflare-pages-project.py" in workflow
    assert '--project "$RELEASE_CHANNEL_PROJECT"' in workflow
    assert workflow.index("Require Cloudflare credentials") < workflow.index(
        "Verify Cloudflare Pages project"
    )
    assert workflow.index("Verify Cloudflare Pages project") < workflow.index(
        "cloudflare/wrangler-action@"
    )
    assert "pages deploy target/distribution/ --project-name=release" in workflow
    assert "assets/stable/manifest.json" not in workflow
    assert (
        "RELEASE_SITE_URL: ${{ inputs.release_site_url || 'https://release.capsem.org' }}"
        in workflow
    )
    assert "Deploy immutable preview" in workflow
    assert "Validate preview distribution" in workflow
    assert "Activate verified production distribution" in workflow
    assert "Validate activated production bytes" in workflow
    assert "Restore prior production deployment" in workflow
    assert "Verify restored production bytes" in workflow
    assert "uv run --project build_system --frozen python scripts/check-release-site-contract.py" in workflow
    assert '--base-url "$RELEASE_SITE_URL"' in workflow
    assert 'CHANNEL_ARGS=(--channel "$CHANNEL")' in workflow
    assert "CHANNEL_ARGS=(--catalog-members)" in workflow
    assert '"${CHANNEL_ARGS[@]}"' in workflow
    assert "--attempts 30" in workflow
    assert "--delay-seconds 20" in workflow
    assert workflow.index("Deploy immutable preview") < workflow.index(
        "Validate preview distribution"
    )
    assert workflow.index("Validate preview distribution") < workflow.index(
        "Activate verified production distribution"
    )
    assert workflow.index("Activate verified production distribution") < workflow.index(
        "Validate activated production bytes"
    )


def test_release_channel_deploy_runs_python_contract_validator_after_cloudflare_deploy() -> None:
    workflow = _workflow_text("release-channel.yaml")
    validator_step = workflow.split("- name: Validate activated production bytes", maxsplit=1)[
        1
    ].split("\n      - name:", maxsplit=1)[0]

    assert "Validate activated production bytes" in workflow
    assert "uv run --project build_system --frozen python scripts/check-release-site-contract.py" in validator_step
    assert '--base-url "$RELEASE_SITE_URL"' in validator_step
    assert "--catalog-members" in validator_step
    assert 'CHANNEL_ARGS=(--channel "$CHANNEL")' in validator_step
    assert '"${CHANNEL_ARGS[@]}"' in validator_step
    assert "--attempts 30" in validator_step
    assert "--delay-seconds 20" in validator_step
    assert "--expect-snapshot target/release-channel-deployment/candidate-release.json" in (
        validator_step
    )
    assert workflow.index("Activate verified production distribution") < workflow.index(
        "Validate activated production bytes"
    )


def test_release_channel_staging_workflow_exercises_reusable_deploy_without_release_builds() -> (
    None
):
    workflow = _workflow_text("release-channel-staging.yaml")
    reusable = _workflow_text("release-channel.yaml")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = _skill_text("skills/release-process/SKILL.md")
    asset_skill = _skill_text("skills/asset-pipeline/SKILL.md")

    assert "workflow_dispatch:" in workflow
    assert "default: staging" in workflow
    assert "default: https://staging.release-eq7.pages.dev" in workflow
    assert "_build-assets:" not in workflow
    assert "build-app-macos:" not in workflow
    assert "build-app-linux:" not in workflow
    assert "just _build-kernel" not in workflow
    assert "just _build-rootfs" not in workflow
    assert "scripts/rehearse-asset-channel-staging.sh" in workflow
    assert "--without-binary-files" not in workflow
    assert '"$RUNNER_TEMP/release-channel-staging-fixture"' in workflow
    assert '"$RUNNER_TEMP/release-channel-staging-validation"' in workflow
    assert "--asset-source-base" not in workflow
    assert "scripts/write-release-site-ci-fixture.py" not in workflow
    assert "bash scripts/check-web-surface.sh release-site-build" not in workflow
    assert "cargo run -p capsem-admin -- assets channel check" not in workflow
    assert "name: asset-channel-staging-preview" in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: asset-channel-staging-preview" in workflow
    assert "deploy_branch: ${{ inputs.deploy_branch }}" in workflow
    assert "release_site_url: ${{ inputs.release_site_url }}" in workflow
    assert "activate_production: false" in workflow
    assert (
        "pages deploy target/distribution/ --project-name=release "
        "--branch=${{ inputs.activate_production && format('capsem-preview-{0}-{1}', github.run_id, github.run_attempt) || inputs.deploy_branch }}"
    ) in reusable

    for text in (docs, release_skill, asset_skill):
        assert "release-channel-staging.yaml" in text
        assert (
            "without invoking `build-assets`" in text
            or "without invoking VM asset builds" in text
            or "without invoking profile builders" in text
        )


def test_release_site_contract_script_fails_on_content_drift(capsys) -> None:
    validator = _release_site_contract_module()

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            assert channel == "stable"
            return SimpleNamespace(
                ok=False,
                name="release.capsem.org contract",
                detail=(
                    "health asset hash mismatch for /assets/releases/2030.0101.1/arm64-vmlinuz"
                ),
            )

    exit_code = validator.validate_release_site(
        release_site="https://release.capsem.org",
        channel="stable",
        attempts=1,
        delay_seconds=0,
        checker=FakeChecker,
    )

    captured = capsys.readouterr()
    assert exit_code == 1
    assert "health asset hash mismatch" in captured.err
    assert "arm64-vmlinuz" in captured.err


def test_release_site_contract_cli_validates_each_requested_channel(monkeypatch, capsys) -> None:
    validator = _release_site_contract_module()
    checked_channels: list[str] = []

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            checked_channels.append(channel)
            return SimpleNamespace(
                ok=True,
                name="release.capsem.org contract",
                detail=f"{channel} ok",
            )

    monkeypatch.setattr(validator, "load_readiness_checker", lambda: FakeChecker)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "check-release-site-contract.py",
            "--base-url",
            "https://release.capsem.org",
            "--channel",
            "stable",
            "--channel",
            "nightly",
            "--attempts",
            "1",
            "--delay-seconds",
            "0",
        ],
    )

    exit_code = validator.main()

    captured = capsys.readouterr()
    assert exit_code == 0
    assert checked_channels == ["stable", "nightly"]
    assert "stable release-channel contract passed" in captured.out
    assert "nightly release-channel contract passed" in captured.out


def test_release_site_contract_cli_retries_requested_channels_as_a_set(monkeypatch, capsys) -> None:
    validator = _release_site_contract_module()
    checks: list[str] = []
    sleep_calls: list[float] = []

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            checks.append(channel)
            if checks == ["stable"]:
                return SimpleNamespace(
                    ok=False,
                    name="release.capsem.org contract",
                    detail="stable package page still serving previous deploy",
                )
            return SimpleNamespace(
                ok=True,
                name="release.capsem.org contract",
                detail=f"{channel} ok",
            )

    monkeypatch.setattr(validator, "load_readiness_checker", lambda: FakeChecker)
    monkeypatch.setattr(validator.time, "sleep", sleep_calls.append)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "check-release-site-contract.py",
            "--base-url",
            "https://release.capsem.org",
            "--channel",
            "stable",
            "--channel",
            "nightly",
            "--attempts",
            "2",
            "--delay-seconds",
            "7",
        ],
    )

    exit_code = validator.main()

    captured = capsys.readouterr()
    assert exit_code == 0
    assert checks == ["stable", "nightly", "stable", "nightly"]
    assert sleep_calls == [7]
    assert "attempt 1/2: stable" in captured.err
    assert "stable release-channel contract passed" in captured.out
    assert "nightly release-channel contract passed" in captured.out


def test_release_site_contract_retries_refresh_site_cache_but_reuse_external_bytes(
    monkeypatch, capsys
) -> None:
    validator = _release_site_contract_module()
    cache_clears = 0
    checks: list[tuple[int, str]] = []
    sleep_calls: list[float] = []
    site = "https://release.capsem.org"
    external = "https://github.example.test/release/immutable-rootfs.erofs"

    class CountingCache(dict):
        def clear(self) -> None:
            nonlocal cache_clears
            cache_clears += 1
            super().clear()

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None
        _FETCH_BYTES_CACHE = CountingCache(
            {
                f"{site}/assets/nightly/manifest.json": SimpleNamespace(data=b"old", error=None),
                external: SimpleNamespace(data=b"immutable graph bytes", error=None),
            }
        )

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == site
            assert external in FakeChecker._FETCH_BYTES_CACHE
            checks.append((cache_clears, channel))
            if channel == "nightly" and cache_clears == 1:
                FakeChecker._FETCH_BYTES_CACHE[f"{site}/assets/nightly/manifest.json"] = (
                    SimpleNamespace(data=b"stale", error=None)
                )
                return SimpleNamespace(
                    ok=False,
                    name="release.capsem.org contract",
                    detail="channel manifest SHA-256 mismatch",
                )
            assert f"{site}/assets/nightly/manifest.json" not in FakeChecker._FETCH_BYTES_CACHE
            return SimpleNamespace(
                ok=True,
                name="release.capsem.org contract",
                detail=f"{channel} ok",
            )

    monkeypatch.setattr(validator.time, "sleep", sleep_calls.append)

    exit_code = validator.validate_release_channels(
        release_site="https://release.capsem.org",
        channels=["stable", "nightly"],
        attempts=2,
        delay_seconds=3,
        checker=FakeChecker,
    )

    captured = capsys.readouterr()
    assert exit_code == 0
    assert cache_clears == 2
    assert {
        external: SimpleNamespace(data=b"immutable graph bytes", error=None)
    } == FakeChecker._FETCH_BYTES_CACHE
    assert checks == [
        (1, "stable"),
        (1, "nightly"),
        (2, "stable"),
        (2, "nightly"),
    ]
    assert sleep_calls == [3]
    assert "attempt 1/2: nightly" in captured.err
    assert "stable release-channel contract passed" in captured.out
    assert "nightly release-channel contract passed" in captured.out


def test_release_channel_cloudflare_prerequisites_are_documented() -> None:
    workflow = _workflow_text("release-channel.yaml")
    release_assets = _workflow_text("release-assets.yaml")
    checker = _source_text("scripts/check-cloudflare-pages-project.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = _skill_text("skills/release-process/SKILL.md")
    asset_skill = _skill_text("skills/asset-pipeline/SKILL.md")

    for required in (
        "CLOUDFLARE_ACCOUNT_ID",
        "CLOUDFLARE_API_TOKEN",
        "release",
        "release.capsem.org",
    ):
        assert required in workflow
        assert required in release_assets
        assert required in checker
        assert required in docs
        assert required in release_skill
        assert required in asset_skill

    docs_text = " ".join(docs.split())
    release_skill_text = " ".join(release_skill.split())
    asset_skill_text = " ".join(asset_skill.split())
    for text in (docs_text, release_skill_text, asset_skill_text):
        text_lower = text.lower()
        assert "Release-channel Cloudflare prerequisites" in text
        assert "Pages project serving `release.capsem.org`" in text
        assert "`release.capsem.org` custom domain" in text
        assert "`CLOUDFLARE_ACCOUNT_ID`" in text
        assert "`CLOUDFLARE_API_TOKEN`" in text
        assert "`scripts/check-release-site-contract.py`" in text
        assert "BLAKE3/SHA-256" in text
        assert "cache headers" in text_lower
        assert "rather than only checking that files exist" in text_lower
        assert "before running a live binary or" in text_lower
        assert "channel deploy" in text_lower


def test_cloudflare_pages_project_checker_reports_visibility_failures() -> None:
    checker = _cloudflare_pages_project_module()

    ok, detail = checker.validate_project_response(
        checker.CloudflareResponse(
            200,
            {"success": True, "result": {"name": "release"}},
        ),
        "release",
    )
    assert ok is True
    assert "release is visible" in detail

    ok, detail = checker.validate_project_response(
        checker.CloudflareResponse(
            404,
            {
                "success": False,
                "errors": [
                    {
                        "code": 8000007,
                        "message": (
                            "Project not found. The specified project name does not "
                            "match any of your existing projects."
                        ),
                    }
                ],
            },
        ),
        "release",
    )
    assert ok is False
    assert "Cloudflare Pages project release is not visible" in detail
    assert "8000007: Project not found" in detail
    assert "CLOUDFLARE_ACCOUNT_ID/API_TOKEN" in detail


def test_asset_channel_deploy_smoke_verifies_public_evidence_artifacts() -> None:
    workflow = _workflow_text("release-channel.yaml")
    script = _source_text("scripts/check-remote-release-readiness.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "astral-sh/setup-uv@" in workflow
    assert "uv run --project build_system --frozen python scripts/check-release-site-contract.py" in workflow
    assert "import hashlib" in script
    assert "import blake3" in script
    assert "def fetch_and_verify_evidence_artifact" in script
    assert 'site, sbom, "sha256", "host SBOM evidence", "spdx"' in script
    assert 'site, obom, "blake3", "VM OBOM evidence", "rootfs_cyclonedx"' in script
    assert "data = fetch_bytes(url)" in script
    assert "health evidence host_sboms missing for published binary files" in script
    assert "health evidence vm_oboms missing for published VM assets" in script
    assert "health evidence attestations missing for published artifacts" in script
    assert "attestation_predicate_evidence_urls" in script
    assert "attestation predicate_url {predicate_url} missing from {predicate_label}" in script
    assert "attestation subject {subject} missing from published file lists" in script
    assert "resolves published host SBOM and VM OBOM evidence artifacts from the graph" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates their SPDX 2.3 or CycloneDX document shape" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text
    assert "Profile image attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_docs_preserve_vm_obom_attestation_predicate_contract() -> None:
    docs_text = " ".join(_source_text("docs/src/content/docs/development/ci.md").split())

    assert "Profile image attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_architecture_docs_preserve_vm_obom_attestation_predicate_contract() -> None:
    docs_text = " ".join(
        _source_text("docs/src/content/docs/architecture/asset-pipeline.md").split()
    )

    assert "SBOM and VM OBOM evidence" in docs_text
    assert "VM asset attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_release_channel_cache_header_documentation_matches_deploy_smoke() -> None:
    workflow = _workflow_text("release-channel.yaml")
    ci_docs = _source_text("docs/src/content/docs/development/ci.md")
    architecture_docs = _source_text("docs/src/content/docs/architecture/asset-pipeline.md")
    release_skill = _skill_text("skills/release-process/SKILL.md")
    asset_skill = _skill_text("skills/asset-pipeline/SKILL.md")

    script = _source_text("scripts/check-remote-release-readiness.py")
    assert "uv run --project build_system --frozen python scripts/check-release-site-contract.py" in workflow
    assert "def check_release_cache_headers" in script
    assert '("no-cache", "must-revalidate")' in script
    assert '("public", "max-age=31536000", "immutable")' in script

    for source in [ci_docs, architecture_docs, release_skill, asset_skill]:
        normalized = " ".join(source.split())
        assert "Cache-Control" in source
        assert "no-cache" in source
        assert "must-revalidate" in source
        assert "public, max-age=31536000, immutable" in source
        assert "mutable" in normalized
        assert "immutable" in normalized
        assert "release-channel" in normalized


def test_cdxgen_is_owned_only_by_the_digest_pinned_asset_helper() -> None:
    release_preflight = _source_text("scripts/check-release-workflow.sh")
    doctor = _source_text("scripts/doctor-common.sh")
    asset_workflow = _workflow_text("release-assets.yaml")
    docs_and_skills = [
        _source_text("docs/src/content/docs/development/getting-started.md"),
        _source_text("docs/src/content/docs/development/ci.md"),
        _source_text("skills/dev-start/SKILL.md"),
        _source_text("skills/dev-setup/SKILL.md"),
    ]

    assert "cdxgen" not in release_preflight
    assert "for tool in gh openssl; do" in doctor
    assert "capsem_gate_cargo_tool_versions" in doctor
    assert 'name = "cargo-sbom"' in _source_text("config/gate.toml")
    assert 'skip "$tool (only needed for releases)"' in doctor
    assert "npm install -g @cyclonedx/cdxgen" not in asset_workflow
    assert "CAPSEM_CDXGEN_CMD" not in asset_workflow
    assert 'elif [ "$PLATFORM" = "Darwin" ]' in release_preflight
    assert 'dockerfile = "build_system/docker/Dockerfile.asset-tools"' in _source_text(
        "config/docker/image/build.toml"
    )

    for source in docs_and_skills:
        normalized = " ".join(source.split())
        assert "cdxgen" in source
        assert "helper" in normalized.lower()
        assert "npm install -g @cyclonedx/cdxgen" not in source


def test_release_workflow_preflight_preserves_macos_key_and_linux_skip() -> None:
    preflight = _source_text("scripts/check-release-workflow.sh")

    assert "PLATFORM=$(uname -s)" in preflight
    assert 'if [ "$PLATFORM" = "Darwin" ]' in preflight
    assert 'fail "$KEY_FILE not found"' in preflight
    assert "is macOS signing material and is not applicable" in preflight
    assert "key decodes to valid Tauri updater key format" in preflight
    assert "key does not decode to valid Tauri updater key format" in preflight


def test_linux_doctor_installs_musl_c_toolchain_before_building_assets() -> None:
    doctor = _source_text("scripts/doctor-common.sh")
    linux = _source_text("scripts/doctor-linux.sh")

    assert '_reg linux-musl-tools "_doctor_install_linux_musl_tools"' in doctor
    assert doctor.index("_reg linux-musl-tools") < doctor.index("_reg build-assets")
    assert "check_linux_musl_toolchain" in doctor
    assert 'section "C Toolchain"' in linux
    assert "linux_musl_toolchain_available" in linux
    assert "command -v musl-gcc" in linux
    assert "command -v x86_64-linux-musl-gcc" not in linux
    assert "apt-get install -y musl-tools" in linux
    assert "dnf install -y musl-gcc" in linux


def test_linux_doctor_accepts_native_musl_gcc_without_x86_cross_compiler(
    tmp_path: Path,
) -> None:
    musl_gcc = tmp_path / "musl-gcc"
    musl_gcc.write_text("#!/bin/sh\nexit 0\n")
    musl_gcc.chmod(0o755)

    result = subprocess.run(
        [
            "/bin/bash",
            "-c",
            """
            source scripts/doctor-linux.sh
            section() { :; }
            pass() { printf 'PASS:%s\\n' "$1"; }
            fixable() { printf 'FIXABLE:%s\\n' "$*"; }
            check_linux_musl_toolchain
            """,
        ],
        cwd=PROJECT_ROOT,
        env={"PATH": str(tmp_path)},
        capture_output=True,
        text=True,
        check=True,
    )

    assert result.stdout == "PASS:musl-gcc\n"


def test_cross_surface_update_smoke_prerequisites_are_covered_locally() -> None:
    cli = _source_text("crates/capsem/src/update.rs")
    cli_status = _source_text("crates/capsem/src/tests.rs")
    service = _source_text("crates/capsem-service/src/tests.rs")
    tray = _source_text("crates/capsem-tray/src/menu/tests.rs")
    tui = _source_text("crates/capsem-tui/src/tests.rs")
    frontend = _source_text("web/app/src/lib/__tests__/update-status.test.ts")
    frontend_api = _source_text("web/app/src/lib/__tests__/api.test.ts")

    assert "Profile catalog update available" in cli
    assert "Run `capsem update --assets` separately to refresh VM assets." not in cli
    assert "--assets cannot be combined with --corp" in cli
    assert "update_status_lines_separate_available_and_blocked_tracks" in cli_status
    assert "available (binary" in cli_status
    assert "blocked (assets, images)" in cli_status

    assert "update_route_apply_dry_run_plans_one_atomic_update" in service
    assert "update_route_apply_confirmed_dispatches_one_atomic_update" in service
    assert "update_route_apply_rejects_obsolete_split_action_body" in service
    assert 'json!(["update", "--yes"])' in service
    assert 'json!(["update", "--assets"])' not in service

    assert "spec_mixed_binary_and_asset_updates_share_indicator" in tray
    assert "spec_blocked_profile_update_shows_blocked_indicator" in tray
    assert "spec_blocked_asset_update_shows_blocked_indicator" in tray
    assert "Updates: Binary, VM assets" in tray
    assert "Updates: Binary; blocked: Profiles" in tray

    assert "tui_update_smoke_matrix_covers_release_states_and_atomic_action" in tui
    for case in [
        "binary-update",
        "profile-update",
        "asset-update",
        "mixed-binary-asset-update",
    ]:
        assert case in tui
    assert "ControlAction::Update" in tui
    assert "ControlAction::Update { assets:" not in tui
    assert "update --assets" not in tui

    assert "summarizes mixed binary and VM asset updates without profile noise" in frontend
    assert "treats profile catalog updates as a first-class available track" in frontend
    assert (
        "keeps blocked profile dashboard tracks visible beside available asset tracks" in frontend
    )
    assert "Binary, VM assets available" in frontend
    assert "VM assets available for future sessions" in frontend
    assert "apply the verified VM asset update automatically" in frontend
    assert "capsem update --assets" not in frontend

    assert "applies the complete update transaction through one confirmed body" in frontend_api
    assert (
        "plans the complete update transaction without confirmation only through dry run"
        in frontend_api
    )
    assert "api.applyUpdate({ confirmed: true })" in frontend_api
    assert "applyUpdateAction" not in frontend_api


def test_installed_service_owns_one_serial_automatic_update_path() -> None:
    service = _source_text("crates/capsem-service/src/main.rs")
    update_command = _source_text("crates/capsem-service/src/update_command.rs")
    api = _source_text("crates/capsem-service/src/api.rs")
    route_tests = _source_text("tests/capsem-service/test_update_routes.py")
    apply_request = api.split("pub struct UpdateApplyRequest", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]

    check_executor = service.split("async fn execute_update_command(", maxsplit=1)[1].split(
        "async fn execute_update_apply(", maxsplit=1
    )[0]
    apply_executor = service.split("async fn execute_update_apply(", maxsplit=1)[1].split(
        "async fn execute_update_command_unlocked(", maxsplit=1
    )[0]
    automatic_executor = service.split("async fn run_automatic_update_once(", maxsplit=1)[1].split(
        "async fn run_automatic_update_loop(", maxsplit=1
    )[0]

    assert "update_lock: tokio::sync::Mutex<()>" in service
    assert "run_automatic_update_loop(state_for_updates)" in service
    assert "automatic_updates_enabled()" in automatic_executor
    assert "state.update_lock.try_lock()" in automatic_executor
    assert "CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS" in service
    assert "CAPSEM_AUTOMATIC_UPDATE_POLL_SECS" in service
    assert "automatic_update_delay(" in service
    assert "AUTOMATIC_UPDATE_INITIAL_DELAY_SECS: u64 = 60" in service
    assert "AUTOMATIC_UPDATE_POLL_SECS: u64 = 60 * 60" in service
    assert "state.update_lock.lock().await" in check_executor
    assert "state.update_lock.lock().await" in apply_executor
    assert service.count("execute_update_command_unlocked(plan).await") == 3

    assert "UpdateCommandKind::Apply" in service
    assert 'vec!["update".to_string(), "--yes".to_string()]' in update_command
    assert 'std::env::var_os("INVOCATION_ID")' in update_command
    assert 'std::env::var_os("SYSTEMD_EXEC_PID")' in update_command
    assert "std::process::id()" in update_command
    assert "UpdateCommandKind::Assets" not in update_command
    assert '"--assets".to_string()' not in update_command
    assert "UpdateApplyAction" not in api
    assert "action" not in apply_request
    assert '["update", "--yes"]' in route_tests
    assert '["update", "--assets"]' not in route_tests


def test_docs_and_marketing_sites_build_on_pr_and_deploy_on_main_only() -> None:
    expectations = [
        (
            "docs.yaml",
            "docs",
            "docs-build",
            "capsem-docs",
            "Smoke public docs site",
            "https://docs.capsem.org",
            "docs.capsem.org smoke failed after deploy.",
        ),
        (
            "site.yaml",
            "site",
            "site-build",
            "capsem",
            "Smoke public marketing site",
            "https://capsem.org",
            "capsem.org smoke failed after deploy.",
        ),
    ]

    for (
        workflow_name,
        directory,
        ci_job,
        project_name,
        smoke_name,
        site_url,
        failure,
    ) in expectations:
        workflow_path = PROJECT_ROOT / ".github" / "workflows" / workflow_name
        workflow = workflow_reachable_text(PROJECT_ROOT, workflow_path)
        trigger = workflow.split("\njobs:", maxsplit=1)[0]
        push_trigger = trigger.split("  push:", maxsplit=1)[1]
        ci_block = _workflow_job_block(ci_job)

        assert "pull_request:" not in trigger, workflow_name
        assert "push:" in workflow, workflow_name
        assert "branches: [main]" in workflow, workflow_name
        assert "paths:" in push_trigger, workflow_name
        assert f"cache-dependency-path: {directory}/pnpm-lock.yaml" in ci_block
        assert f"cd {directory} && pnpm install --frozen-lockfile" in ci_block
        assert f"bash scripts/check-web-surface.sh {directory}" in ci_block
        assert frozenset(_workflow_job("pr-gate")["needs"]) == REQUIRED_PR_GATE_JOBS
        assert f"cd {directory} && pnpm install --frozen-lockfile" in workflow
        assert f"bash scripts/check-web-surface.sh {directory}" in workflow
        assert (
            "if: ${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}" in workflow
        )
        assert f"pages deploy {directory}/dist/ --project-name={project_name}" in workflow
        assert smoke_name in workflow
        assert f"SITE_URL: {site_url}" in workflow
        assert 'curl -fsSLI "$SITE_URL/" -o' in workflow
        assert "grep -qi '^content-type: text/html'" in workflow
        assert "grep -qi '<main[ >]'" in workflow or "grep -q '<main id=\"main\">'" in workflow
        assert failure in workflow
        assert "release-channel.yaml" not in workflow
        assert "release.yaml" not in workflow
        assert "release-assets.yaml" not in workflow


def test_binary_release_uses_asset_channel_and_does_not_publish_vm_assets() -> None:
    workflow = _workflow_text("release.yaml")
    fast_gate = _workflow_text("fast-gate.yaml")
    author_candidate = _workflow_job_block("author-binary-candidate", "release.yaml")
    create_release = _workflow_job_block("create-release", "release.yaml")
    assemble_channel = _workflow_job_block("assemble-release-channel", "release.yaml")
    trigger = workflow.split("\npermissions:", maxsplit=1)[0]

    assert "workflow_dispatch:" in trigger
    assert "tag:" in trigger
    assert "channel:" in trigger
    assert "type: choice" in trigger
    assert "options:" in trigger
    assert "- stable" in trigger
    assert "- nightly" in trigger
    assert "run-name: Release ${{ inputs.channel }} ${{ inputs.tag }}" in workflow
    assert "deployments: write" in workflow
    assert "push:" not in trigger
    assert "pull_request:" not in trigger
    assert "branches:" not in trigger
    assert "group: capsem-release-${{ inputs.channel }}" in workflow
    assert "cancel-in-progress: false" in workflow
    assert "RELEASE_TAG: ${{ inputs.tag }}" in workflow
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert (
        "ASSET_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json"
        in workflow
    )
    assert "Verify immutable source ref, version tag, and channel" in workflow
    assert 'test "$GITHUB_REF_TYPE" = tag' in workflow
    assert 'test "$GITHUB_REF_NAME" = "capsem-source-$SOURCE_COMMIT"' in workflow
    assert 'case "$RELEASE_TAG" in v*) ;; *) exit 1 ;; esac' in workflow
    assert "BINARY_RELEASE_CHANNELS" not in workflow
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
    assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate
    assert "just qualify-binaries" in workflow
    assert "just _test-release-contracts" not in workflow
    assert "just _build-kernel" not in workflow
    assert "just _build-rootfs" not in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" not in workflow
    assert "generate_checksums(Path('unified-assets')" not in workflow
    assert 'gh release upload ${{ github.ref_name }} "release-artifacts/$arch' not in workflow
    assert "release-artifacts/manifest.json" not in workflow
    assert "assets-v{asset_version}" in workflow
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "release.capsem.org" in workflow
    assert "assets channel record-binary" in workflow
    assert (
        '--asset-source-base "https://github.com/${{ github.repository }}/releases/download/assets-v{asset_version}"'
        in workflow
    )
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: binary-channel-preview" in workflow
    assert "needs: [deploy-release-channel]" in workflow
    assert "cloudflare/wrangler-action" not in workflow
    assert "pages deploy" not in workflow
    assert "tests/capsem-release/test_binary_lane_gate.py" in workflow
    assert "tests/capsem-release/test_release_lane_diff_policy.py" in workflow
    assert "CLOUDFLARE_" not in workflow
    for logical_name in (
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
        "obom.cdx.json",
        "software-inventory.json",
    ):
        assert f"release-artifacts/{logical_name}" not in create_release
        assert f"release-artifacts/*{logical_name}" not in create_release
    assert "release-artifacts/*.pkg" in create_release
    assert "release-artifacts/*.deb" in create_release
    assert "release-artifacts/capsem-sbom.spdx.json" in create_release
    assert "scripts/publish-immutable-release-assets.sh" in create_release
    assert 'CAPSEM_RELEASE_CREATE_TITLE="Capsem $RELEASE_TAG ($SOURCE_COMMIT)"' in create_release
    assert 'CAPSEM_RELEASE_CREATE_NOTES_FILE="$notes"' in create_release
    assert 'CAPSEM_RELEASE_CREATE_TARGET="$SOURCE_COMMIT"' in create_release
    assert "gh release create" not in create_release
    assert "gh release upload" not in create_release
    immutable_publisher = (
        PROJECT_ROOT / "scripts" / "publish-immutable-release-assets.sh"
    ).read_text()
    assert 'gh release create "$release_tag"' in immutable_publisher
    assert 'gh release upload "$release_tag" "$owned_dir/$missing"' in immutable_publisher
    assert "target/binary-channel/$RELEASE_CHANNEL/manifest.json" in assemble_channel
    assert "target/binary-channel/$RELEASE_CHANNEL/manifest.before.json" in assemble_channel
    assert "name: binary-channel-candidate" in assemble_channel
    assert "path: target/binary-channel/" in assemble_channel
    record_step = author_candidate.split(
        "- name: Record binary candidate metadata once", maxsplit=1
    )[1].split("- name: Prove binary candidate preserved every profile", maxsplit=1)[0]
    assert "target/binary-channel/$RELEASE_CHANNEL/manifest.json" in record_step
    assert "for channel in" not in record_step
    build_channels = assemble_channel.split(
        "- name: Build complete release channels with existing VM assets", maxsplit=1
    )[1].split("- uses: actions/upload-artifact", maxsplit=1)[0]
    assert "generated_at=\"$(date -u +'%Y-%m-%dT%H:%M:%SZ')\"" in build_channels
    assert '--generated-at "$generated_at"' in build_channels
    assert "scripts/build-complete-release-channel.py" in build_channels
    assert (
        '--channel-source "$RELEASE_CHANNEL=file://$PWD/target/binary-channel/$RELEASE_CHANNEL/manifest.json"'
        in build_channels
    )
    assert '--primary-channel "$RELEASE_CHANNEL"' in build_channels
    assert build_channels.index('generated_at="$(date -u') < build_channels.index(
        "scripts/build-complete-release-channel.py"
    )
    assert "Prove binary candidate preserved every profile" in author_candidate
    assert "binary candidate changed profile metadata" in author_candidate
    assert author_candidate.index("Preserve serialized public-before manifest") < (
        author_candidate.index("Record binary candidate metadata once")
    )
    assert workflow.index("Record binary candidate metadata once") < workflow.index(
        "Qualify the candidate binaries"
    )
    assert "Record binary candidate metadata once" not in assemble_channel
    assert "- name: Build release site pages" not in assemble_channel
    assert "- name: Check binary-updated release channels" not in assemble_channel


def test_binary_release_channel_assembly_preflights_canonical_artifacts() -> None:
    author_candidate = _workflow_job_block("author-binary-candidate", "release.yaml")
    assemble_channel = _workflow_job_block("assemble-release-channel", "release.yaml")

    assert "Verify binary candidate artifacts" in author_candidate
    assert "release-artifacts/capsem-sbom.spdx.json" in author_candidate
    assert "release-artifacts/*.pkg" in author_candidate
    assert "release-artifacts/*.deb" in author_candidate
    assert author_candidate.index("Verify binary candidate artifacts") < (
        author_candidate.index("Record binary candidate metadata once")
    )
    assert "Verify tested binary candidate source is present" in assemble_channel
    assert "manifest.before.json" in assemble_channel
    assert "manifest.json" in assemble_channel
    assert "release-artifacts/" not in assemble_channel


def test_binary_release_staging_dry_run_is_separate_from_tag_release() -> None:
    workflow = _workflow_text("release-binary-staging.yaml")
    real_release = _workflow_text("release.yaml")
    macos_ci = _workflow_job_block("test", "ci.yaml")
    artifact_builder = _source_text("scripts/write-binary-staging-artifacts.sh")
    complete_builder = _source_text("scripts/build-complete-release-channel.py")
    compact_complete_builder = " ".join(complete_builder.split())
    assemble_channel = _workflow_job_block(
        "assemble-binary-channel",
        "release-binary-staging.yaml",
    )

    real_trigger = real_release.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" in real_trigger
    assert "push:" not in real_trigger
    assert "tag:" in real_trigger
    assert "channel:" in real_trigger

    assert "workflow_dispatch:" in workflow
    assert "asset_channel:" in workflow
    assert "description: Existing VM asset channel to use as the staging source." in workflow
    assert "default: stable" not in workflow.split("\npermissions:", maxsplit=1)[0]
    assert "push:" not in workflow
    assert "tags:" not in workflow
    assert "finalize-binary-staging-fixtures.py" in artifact_builder
    assert "touch -h -d" not in artifact_builder
    assert "tar --sort" not in artifact_builder
    assert "dpkg-deb" not in artifact_builder
    assert "macOS release portability preflight" in macos_ci
    assert "test_binary_staging_artifacts_are_deterministic_and_recordable" in macos_ci
    assert macos_ci.index("macOS release portability preflight") < macos_ci.index(
        "Unit tests with coverage"
    )
    assert "contents: read" in workflow
    assert "deployments: write" not in workflow
    assert "secrets: inherit" not in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" not in workflow
    assert "pages deploy" not in workflow
    assert "gh release create" not in workflow
    assert "gh release upload" not in workflow
    assert "just _build-kernel" not in workflow
    assert "just _build-rootfs" not in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" not in workflow
    assert "_build-assets:" not in workflow
    for logical_name in (
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
        "obom.cdx.json",
        "software-inventory.json",
    ):
        assert f"release-artifacts/{logical_name}" not in workflow
        assert f"release-artifacts/*{logical_name}" not in workflow

    assert "ASSET_MANIFEST_URL:" not in workflow
    assert 'case "$ASSET_CHANNEL" in stable|nightly)' in assemble_channel
    assert "scripts/fetch-channel-source-manifest.py" in assemble_channel
    assert '--channel "$ASSET_CHANNEL"' in assemble_channel
    assert "--require-profile-membership" in assemble_channel
    assert "scripts/write-binary-staging-artifacts.sh" in assemble_channel
    assert "Capsem-${VERSION}.pkg" in artifact_builder
    assert "Capsem_${VERSION}_arm64.deb" in artifact_builder
    assert "capsem-sbom.spdx.json" in artifact_builder
    assert "Record binary release metadata in channel manifest" in assemble_channel
    assert "assets channel record-binary" in assemble_channel
    assert "ref: ${{ github.sha }}" in assemble_channel
    assert '--source-commit "${{ github.sha }}"' in assemble_channel
    assert "manifest.before.json" in assemble_channel
    assert "scripts/write-binary-channel-staging-proof.py" in assemble_channel
    staging_proof = _source_text("scripts/write-binary-channel-staging-proof.py")
    assert "binary dry-run changed profile image metadata" in staging_proof
    assert "binary dry-run changed VM asset metadata" in staging_proof
    assert '"vm_asset_jobs": "not_run"' in staging_proof
    assert '"vm_assets_unchanged": True' in staging_proof
    assert "Build complete binary channel preview with existing VM assets" in assemble_channel
    assert "scripts/build-complete-release-channel.py" in assemble_channel
    assert "assets channel build" not in assemble_channel
    assert "assets channel check" not in assemble_channel
    assert '"assets", "channel", "build",' in compact_complete_builder
    assert '"assets", "channel", "check",' in compact_complete_builder
    assert "name: binary-channel-dry-run-bundle" in assemble_channel
    assert "${{ runner.temp }}/binary-channel-dry-run/" in assemble_channel
    assert "${{ runner.temp }}/release-channel/" in assemble_channel


def test_binary_release_summary_names_pkg_and_deb_sbom_coverage() -> None:
    """The claim follows the summary into the script that now renders it.

    It was asserted against the workflow because the summary was a shell body
    inside `run:`. That body also carried `[ -n "$LINUX_ROWS" ]` -- a refusal,
    not formatting -- which no test could reach.
    """
    create_release = _workflow_job_block("create-release", "release.yaml")
    assert "scripts/write-release-summary.py" in create_release

    summary = _source_text("scripts/write-release-summary.py")
    assert "SBOM attested (SPDX 2.3, pkg + deb)" in summary
    assert "SBOM attested (SPDX 2.3, pkg)\n" not in summary


def test_binary_release_does_not_publish_latest_json_updater_metadata() -> None:
    workflow = _workflow_text("release.yaml")
    docs = _source_text("docs/src/content/docs/development/ci.md")
    release_skill = _skill_text("skills/release-process/SKILL.md")

    assert "latest.json" not in workflow
    assert "api.github.com/repos/google/capsem/releases/latest" not in workflow
    docs_text = " ".join(docs.split())
    assert "binary freshness comes from the selected manifest in the release graph" in docs_text
    assert "releases do not rebuild or upload profile images, and they do not publish" in docs_text
    assert (
        "`latest.json`; binary freshness comes from the selected manifest in the release graph"
        in docs_text
    )
    assert "`latest.json` is absent in the current release rail" in release_skill
    assert "Do not make release creation depend on `latest.json`" in release_skill


def test_binary_release_channel_policy_supports_daily_nightly_and_explicit_stable() -> None:
    workflow = _workflow_text("release.yaml")
    docs = _source_text("docs/src/content/docs/development/ci.md")
    normalized_docs = " ".join(docs.split())
    release_skill = _skill_text("skills/release-process/SKILL.md")

    trigger = workflow.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" in trigger
    assert "- stable" in trigger
    assert "- nightly" in trigger
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert "group: capsem-release-${{ inputs.channel }}" in workflow
    assert "cancel-in-progress: false" in workflow
    assert "Prove binary candidate preserved every profile" in workflow
    assert "Nightly rebuild runs once daily" in normalized_docs
    assert "Stable has no schedule" in normalized_docs
    assert "Daily nightly automation calls this same binary command path" in release_skill
    assert "Stable uses the same command explicitly" in " ".join(release_skill.split())


def test_release_lanes_reuse_complete_modules_without_independent_sha_authority() -> None:
    binary = _workflow_text("release.yaml")
    profile = _workflow_text("release-assets.yaml")
    fast_gate = _workflow_text("fast-gate.yaml")
    runtime_preflight = _workflow_text("release-runtime-preflight.yaml")
    agents = _source_text("AGENTS.md")
    testing_skill = _source_text("skills/dev-testing/SKILL.md")
    release_skill = _skill_text("skills/release-process/SKILL.md")

    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate

    for workflow, verb in ((binary, "qualify-binaries"), (profile, "qualify-assets")):
        assert "group: capsem-release-${{ inputs.channel }}" in workflow
        assert "cancel-in-progress: false" in workflow
        assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
        assert f"just {verb}" in workflow
        assert "just _test-release-contracts" not in workflow

    assert "uses: ./.github/workflows/release-runtime-preflight.yaml" in binary
    assert "uses: ./.github/workflows/release-runtime-preflight.yaml" in profile
    assert "workflow_call:" in runtime_preflight
    assert "workflow_dispatch:" not in runtime_preflight
    assert "inputs.sha" not in runtime_preflight
    assert "EXPECTED_SHA" not in runtime_preflight
    assert "Local `just test-clean` is the whole-world proof" in release_skill
    assert "Release CI reuses the same checked-in private modules" in testing_skill
    assert "Serialized Orthogonal Releases" in agents


def test_clean_build_pins_sse_stream_api() -> None:
    workspace = _source_text("Cargo.toml")
    server_manager = _source_text("crates/capsem-core/src/mcp/server_manager.rs")

    assert 'sse-stream = "=0.2.4"' in workspace
    assert "SseStream::from_bytes_stream" in server_manager
    assert "SseStream::from_byte_stream" not in server_manager


def test_binary_release_installs_exact_artifacts_before_publication() -> None:
    workflow = _workflow_text("release.yaml")
    macos_build = _workflow_job_block("build-app-macos", "release.yaml")
    linux_build = _workflow_job_block("build-app-linux", "release.yaml")
    author = _workflow_job_block("author-binary-candidate", "release.yaml")
    macos = _workflow_job_block("test-native-macos-package", "release.yaml")
    linux = _workflow_job_block("test-native-linux-package", "release.yaml")
    create_release = _workflow_job_block("create-release", "release.yaml")

    assert "  test-install:" not in workflow
    assert "needs: [preflight, resolve-channel-source]" in macos_build
    assert "needs: [preflight, resolve-channel-source]" in linux_build
    assert "Install exact notarized package" not in macos_build
    assert "Install and verify exact release deb" not in linux_build
    assert "sudo /usr/sbin/installer" not in macos_build
    assert "sudo dpkg -i" not in linux_build
    assert "Build .pkg installer" in macos_build
    assert "Verify macOS package installation path policy" in macos_build
    assert "Notarize and staple .pkg" in macos_build
    assert "Verify exact notarized package identity and Gatekeeper acceptance" in macos_build
    assert "Build Linux package through the shared hermetic rail" in linux_build
    assert "uv run --project build_system --frozen capsem-gate cross-compile" in linux_build
    assert "Collect Linux artifacts" in linux_build
    assert "Record binary candidate metadata once" in author
    assert "Prove binary candidate preserved every profile" in author

    assert "needs: [build-app-macos, author-binary-candidate]" in macos
    assert "needs: [build-app-linux, author-binary-candidate]" in linux
    assert "name: binary-channel-candidate" in macos
    assert "name: binary-channel-candidate" in linux
    assert "PREACTIVATION_MANIFEST=file://" in macos
    assert "PREACTIVATION_MANIFEST=file://" in linux
    assert 'sudo /usr/sbin/installer -pkg "$package" -target /' in macos
    assert "build_system/packaging/shared/install-manifest-request.sh write" in macos
    assert "pkgutil --pkg-info com.capsem.pkg" in macos
    assert 'test -d "/Applications/Capsem.app"' in macos
    assert 'test -x "/Applications/Capsem.app/Contents/MacOS/capsem-app"' in macos
    assert (
        "for bin in capsem capsem-admin capsem-gateway capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-process capsem-service capsem-tray capsem-tui capsem-mock-server capsem-bench-rs"
        in macos
    )
    assert 'grep -F "Installed: true" /tmp/capsem-status.txt' in macos
    assert 'grep -F "Running:   true" /tmp/capsem-status.txt' in macos
    assert 'grep -F "Service:   ok" /tmp/capsem-status.txt' in macos
    assert 'grep -F "Gateway:   ok" /tmp/capsem-status.txt' in macos
    assert "pgrep -x capsem-tray" in macos
    assert "scripts/verify-installed-release.py" in macos
    assert "Collect macOS install diagnostics" in macos
    assert "if: always()" in macos
    assert '"$HOME/.capsem/logs/install-latest.log"' in macos
    assert macos.index("Install and verify exact notarized package") < macos.index(
        "Collect macOS install diagnostics"
    )

    assert "Install and verify exact release deb" in linux
    assert (
        'python3 build_system/packaging/linux/install-deb-runtime-dependencies.py '
        '"$package"' in linux
    )
    assert 'sudo dpkg -i "$package"' in linux
    assert "sudo apt-get install -f -y" not in linux
    assert linux.index("install-deb-runtime-dependencies.py") < linux.index(
        "install-manifest-request.sh write"
    )
    assert (
        "for bin in capsem capsem-admin capsem-app capsem-gateway capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-process capsem-service capsem-tray capsem-tui capsem-mock-server capsem-bench-rs"
        in linux
    )
    assert "dpkg-query -W -f='${Version}' capsem | grep -Fx \"$VERSION\"" in linux
    assert 'grep -F "Installed: true" /tmp/capsem-status.txt' in linux
    assert 'grep -F "Running:   true" /tmp/capsem-status.txt' in linux
    assert 'grep -F "Service:   ok" /tmp/capsem-status.txt' in linux
    assert 'grep -F "Gateway:   ok" /tmp/capsem-status.txt' in linux
    assert "scripts/verify-installed-release.py" in linux
    assert "Enable KVM for exact-package VM proof" in linux
    assert linux.count("if: matrix.arch == 'x86_64'") == 2
    assert "test -r /dev/kvm -a -w /dev/kvm" in linux
    assert "scripts/prove-installed-shell.py" in linux
    assert "--session-name release-exact-shell-x86_64" in linux
    assert "CAPSEM_EXACT_PACKAGE_SHELL_OK" in linux
    assert "/usr/bin/capsem run" not in linux
    assert "run: just test-clean" not in workflow
    assert (
        "needs: [test-native-macos-package, test-native-linux-package, test-binary-pairing]"
        in create_release
    )
    assert "_gate-install" not in create_release
    assert "continue-on-error: true" not in create_release


def test_install_preflight_releases_base_after_derived_image_is_verified() -> None:
    """The exact install-image graph finishes before its rail is released.

    Releasing it earlier makes the rebuild-on-smoke-failure path cold, and
    releasing it never starves the package rails that follow. It used to be a
    statement at the end of the preflight, ordered by the line it sat on --
    which held until the composed plan put the preflight ahead of the parity
    lane, and then the release landed 164ms before `cache-ownership` ran the
    image it had just deleted. It is a step with edges now, so "after" is a
    property of the graph rather than of the file.
    """
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "installplan.py").read_text()
    identity_source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "installimage.py").read_text()

    # The plan now names each boundary separately, so its log can distinguish
    # the sole egress phase from the sealed source build and smoke proof.
    from capsem_builder.gate.installimage import InstallImageStep

    for lifecycle in InstallImageStep:
        assert f"_step_label(InstallImageStep.{lifecycle.name})" in source
    assert "release(" not in source, (
        "the preflight reclaims nothing: the rail belongs to the parity lane, "
        "whose own step hands it back"
    )

    # Nothing releases it early any more, because both package builds run it:
    # `capsem-host-builder`'s own `last_consumer` is `package-x86_64`. It is
    # freed at `after-packages`, after every consumer.
    # Nothing releases it early any more. Both package builds run it, and the
    # install proof's image is `FROM` it, so `after-install` -- released on the
    # way out of that proof -- is the first point at which nothing needs it.
    labels = list(_gate_labels())
    release = labels.index("glowup.install")
    for consumer in (
        "install.capacity",
        "install.materialize",
        "install.image-build",
        "install.image-smoke",
        "cache-ownership",
        "linux-rust",
    ):
        if consumer in labels:
            assert labels.index(consumer) < release
    # The lane is eight phases now; the build is the one that runs the image.
    for package in ("package.arm64.build", "package.x86_64.build"):
        assert labels.index(package) < release, (
            "the package builds run this image; releasing it first is exit 125"
        )

    phase = config.storage.phases["after-install"]
    assert (phase.boundary, phase.rail) == ("after-install", "install")

    # An image that merely exists is not current: every later phase revalidates
    # its input-key label and exact platform child before use.
    assert "require_input_key(" in identity_source
    assert "exact_image_id(" in identity_source
    assert "Building missing capsem-host-builder base image" not in identity_source


def test_release_skill_requires_ci_and_local_mac_installer_outcome_proof() -> None:
    release_skill = _skill_text("skills/release-process/SKILL.md")
    normalized_release_skill = " ".join(release_skill.split())

    assert "Native installation and platform gates" in release_skill
    assert "macOS CI builds the publishable `.pkg`" in release_skill
    assert "Linux CI builds every required `.deb`" in release_skill
    assert "Local Apple Silicon `just test-clean` owns that VZ proof" in release_skill
    assert "Hosted macOS owns signing, notarization" in normalized_release_skill
    assert "publication depends on both platform rails" in release_skill
    assert "Fix forward with a normal commit" in release_skill
    assert "scripts/verify-installed-release.py" in release_skill
    assert "byte-for-byte" in release_skill
    assert "profile readiness" in release_skill


def test_release_skill_requires_exact_manifest_single_metadata_and_shared_status_contract() -> None:
    release_skill = _skill_text("skills/release-process/SKILL.md")
    installation_skill = _source_text("skills/dev-installation/SKILL.md")

    for source in (release_skill, installation_skill):
        normalized = " ".join(source.split())
        assert "assets/manifest.json" in source
        assert "assets/manifest-metadata.json" in source
        assert "capsem.manifest_metadata.v1" in source
        assert "GET /system/status" in source
        assert "in memory" in normalized or "in-memory" in normalized
    release_normalized = " ".join(release_skill.split())
    assert "installed source of truth remains the exact verified" in release_normalized
    assert "must not rewrite it into a reduced runtime schema" in release_normalized
    assert "do not create a separate origin file" in release_normalized
    assert "the UI must not synthesize publication state" in release_normalized


def test_release_dispatch_has_exactly_two_single_purpose_just_recipes() -> None:
    justfile = _source_text("justfile")

    assert '\nrelease tag="" channel="stable":' not in f"\n{justfile}"
    assert "\nprepare-release:" not in justfile
    assert '\nrelease-binaries channel source_commit force="false":' in justfile
    assert '\nrelease-profile channel profile source_commit force="false":' in justfile
    assert "scripts/release-binaries.py" in _recipe_block("release-binaries")
    assert "capsem-admin -- release" in _recipe_block("release-profile")


def test_self_update_docs_match_verified_package_execution() -> None:
    update_rs = _source_text("crates/capsem/src/update.rs")
    install_tests = _source_text("tests/capsem-install/test_update.py")
    install_skill = _source_text("skills/dev-installation/SKILL.md")
    architecture_skill = _skill_text("skills/site-architecture/SKILL.md")
    service_docs = _source_text("docs/src/content/docs/architecture/service-architecture.md")

    assert "apply_binary_installer_plan(&plan).await?" in update_rs
    assert "Binary update applied. Restart Capsem" in update_rs
    assert "test_macos_update_yes_applies_verified_pkg_with_package_manager" in install_tests
    assert "test_linux_update_yes_applies_verified_deb_with_package_manager" in install_tests
    assert "/usr/sbin/installer -pkg {cached} -target /" in install_tests
    assert "apt-get install --yes --allow-downgrades {cached}" in install_tests
    assert "and print the tested package-manager apply command (`sudo" not in install_skill
    assert (
        "downloads verified binary installers, prints the package-manager apply command,"
        not in (architecture_skill)
    )
    assert "prints the\ntested package-manager apply command for the verified package" not in (
        service_docs
    )
    assert "prints the tested package-manager apply command for audit" in install_skill
    assert "executes it through `sudo`" in install_skill
    assert "executes it with `--yes`" in architecture_skill
    assert "executes that command through\n`sudo`" in service_docs


def test_installation_skill_documents_full_host_binary_cohort() -> None:
    install_skill = _source_text("skills/dev-installation/SKILL.md")
    install_fixture = _source_text("tests/capsem-install/conftest.py")

    binaries_match = re.search(r"BINARIES = \[(.*?)\]", install_fixture, re.S)
    assert binaries_match is not None
    binaries = re.findall(r'"([^"]+)"', binaries_match.group(1))
    assert binaries

    for binary in binaries:
        assert binary in install_skill
    assert "all packaged host binaries expose a version surface" in install_skill
    assert "capsem update --yes" in install_skill


def _install_release_graph_contract_fixture(
    checker,
    *,
    index_text: str | None = None,
    channels_mutator=None,
    manifest_mutator=None,
    catalog_mutator=None,
    payload_mutator=None,
    headers_mutator=None,
) -> dict[str, object]:
    site = "https://release.capsem.org"
    channel = "stable"
    current_binary = "1.4.0"
    current_assets = "2030.0101.1"
    profile_revision = "profiles-2030.0101.1"
    manifest_path = "/assets/stable/manifest.json"
    asset_base = "/assets/releases"

    def digest(data: bytes) -> dict[str, str]:
        return {
            "sha256": hashlib.sha256(data).hexdigest(),
            "blake3": checker.blake3.blake3(data).hexdigest(),
        }

    artifacts = {
        "/packages/Capsem-1.4.0.pkg": b"package bytes\n",
        "/packages/Capsem-1.4.0.spdx.json": b'{"spdxVersion":"SPDX-2.3","files":[]}\n',
        f"/profiles/releases/{profile_revision}/co-work/arm64/profile.toml": b'id = "co-work"\n',
        f"/profiles/releases/{profile_revision}/co-work/arm64/software-inventory.json": (
            b'{"schema":"capsem.profile_software_inventory.v1","packages":[]}\n'
        ),
        f"{asset_base}/{current_assets}/arm64-vmlinuz": b"kernel bytes\n",
        f"{asset_base}/{current_assets}/arm64-initrd.img": b"initrd bytes\n",
        f"{asset_base}/{current_assets}/arm64-rootfs.erofs": b"rootfs bytes\n",
        f"{asset_base}/{current_assets}/arm64-obom.cdx.json": (_rootfs_obom_bytes()),
    }

    def file_record(kind: str, name: str, url: str) -> dict[str, object]:
        data = artifacts[url]
        return {
            "kind": kind,
            "name": name,
            "url": url,
            "status": "current",
            "bytes": len(data),
            "digest": digest(data),
        }

    package_url = "/packages/Capsem-1.4.0.pkg"
    package_sbom_url = "/packages/Capsem-1.4.0.spdx.json"
    config_url = f"/profiles/releases/{profile_revision}/co-work/arm64/profile.toml"
    software_inventory_url = (
        f"/profiles/releases/{profile_revision}/co-work/arm64/software-inventory.json"
    )
    obom_url = f"{asset_base}/{current_assets}/arm64-obom.cdx.json"

    manifest = {
        "version": current_binary,
        "status": "current",
        "packages": [
            {
                "id": "capsem-1-4-0-pkg",
                "kind": "macos_pkg",
                "platform": "macos",
                "architecture": "arm64",
                "name": "Capsem-1.4.0.pkg",
                "version": current_binary,
                "url": package_url,
                "bytes": len(artifacts[package_url]),
                "digest": digest(artifacts[package_url]),
                "evidence": [
                    {
                        "kind": "sbom",
                        "url": package_sbom_url,
                        "bytes": len(artifacts[package_sbom_url]),
                        "digest": digest(artifacts[package_sbom_url]),
                    }
                ],
                "binaries": [
                    {
                        "name": "capsem-app",
                        "version": current_binary,
                        "description": "Capsem desktop application executable",
                        "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
                        "architecture": "arm64",
                        "platform": "macos",
                        "bytes": 12,
                        "digest": digest(b"capsem-app binary\n"),
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    }
                ],
            }
        ],
        "profiles": {
            "co-work": {
                "id": "co-work",
                "name": "Co-work",
                "description": "Collaborative agent profile.",
                "revision": profile_revision,
                "min_capsem_version": current_binary,
                "architectures": [
                    {
                        "architecture": "arm64",
                        "software": [
                            {
                                "name": "@openai/codex",
                                "version": "0.142.5",
                                "source": "npm",
                                "architecture": "arm64",
                                "evidence": software_inventory_url,
                                "digest": digest(b"codex software row\n"),
                            }
                        ],
                        "config": [
                            {
                                "kind": "profile",
                                "path": "profiles/co-work/profile.toml",
                                "url": config_url,
                                "status": "current",
                                "bytes": len(artifacts[config_url]),
                                "digest": digest(artifacts[config_url]),
                            }
                        ],
                        "images": [
                            file_record(
                                "kernel",
                                "vmlinuz",
                                f"{asset_base}/{current_assets}/arm64-vmlinuz",
                            ),
                            file_record(
                                "initrd",
                                "initrd.img",
                                f"{asset_base}/{current_assets}/arm64-initrd.img",
                            ),
                            file_record(
                                "rootfs",
                                "rootfs.erofs",
                                f"{asset_base}/{current_assets}/arm64-rootfs.erofs",
                            ),
                        ],
                        "evidence": [
                            {
                                "kind": "software_inventory",
                                "url": software_inventory_url,
                                "status": "current",
                                "bytes": len(artifacts[software_inventory_url]),
                                "digest": digest(artifacts[software_inventory_url]),
                            },
                            {
                                "kind": "obom",
                                "url": obom_url,
                                "status": "current",
                                "bytes": len(artifacts[obom_url]),
                                "digest": digest(artifacts[obom_url]),
                            },
                        ],
                    }
                ],
            }
        },
    }
    if catalog_mutator is not None:
        catalog_mutator(manifest["profiles"]["co-work"])
    if manifest_mutator is not None:
        manifest_mutator(manifest)
    manifest_bytes = (json.dumps(manifest, sort_keys=True) + "\n").encode()
    manifest_digest = digest(manifest_bytes)

    channels = {
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            channel: {
                "label": "Stable",
                "description": "Recommended release channel.",
                "manifests": [
                    {
                        "version": current_binary,
                        "revision": current_binary,
                        "status": "current",
                        "url": manifest_path,
                        "digest": manifest_digest,
                    }
                ],
            }
        },
    }
    if channels_mutator is not None:
        channels_mutator(channels)

    payloads = {f"{site}{manifest_path}": manifest_bytes}
    payloads.update({f"{site}{path}": data for path, data in artifacts.items()})
    if payload_mutator is not None:
        payload_mutator(payloads, checker)

    if index_text is None:
        index_text = " ".join(
            [
                "Stable",
                "Recommended release channel.",
                current_binary,
                manifest_path,
            ]
        )

    package = manifest["packages"][0]
    binary = package["binaries"][0]
    profile = manifest["profiles"]["co-work"]
    architecture = profile["architectures"][0]
    config = architecture["config"][0]
    image_digest_labels = [
        label
        for image in architecture["images"]
        for label in (
            checker.hash_label(image["digest"]["sha256"]),
            checker.hash_label(image["digest"]["blake3"]),
        )
    ]
    evidence_digest_labels = [
        label
        for evidence in architecture["evidence"]
        for label in (
            checker.hash_label(evidence["digest"]["sha256"]),
            checker.hash_label(evidence["digest"]["blake3"]),
        )
    ]
    channel_page_text = " ".join(
        [
            "Stable",
            current_binary,
            manifest_path,
            package["name"],
            package["version"],
            profile["id"],
            profile["name"],
            profile["revision"],
            profile["min_capsem_version"],
        ]
    )
    package_page_text = " ".join(
        [
            package["name"],
            package["version"],
            package["kind"],
            checker.hash_label(package["digest"]["sha256"]),
            checker.hash_label(package["digest"]["blake3"]),
            binary["name"],
            binary["version"],
            binary["description"],
            binary["installed_path"],
            binary["sbom_component_ref"],
            checker.hash_label(binary["digest"]["sha256"]),
            checker.hash_label(binary["digest"]["blake3"]),
        ]
    )
    profile_page_text = " ".join(
        [
            profile["name"],
            profile["id"],
            profile["revision"],
            architecture["architecture"],
            checker.hash_label(config["digest"]["sha256"]),
            checker.hash_label(config["digest"]["blake3"]),
            *image_digest_labels,
            *evidence_digest_labels,
        ]
    )

    headers = {
        f"{site}/": "no-cache, must-revalidate",
        f"{site}/channels.json": "no-cache, must-revalidate",
        f"{site}{manifest_path}": "no-cache, must-revalidate",
    }
    for path in artifacts:
        headers[f"{site}{path}"] = "public, max-age=31536000, immutable"
    if headers_mutator is not None:
        headers_mutator(headers)

    def fake_fetch_text(url: str):
        if url == f"{site}/":
            return checker.FetchText(text=index_text)
        if url == f"{site}/channels/{channel}/":
            return checker.FetchText(text=channel_page_text)
        if url == f"{site}/channels/{channel}/packages/{package['id']}/":
            return checker.FetchText(text=package_page_text)
        if url == f"{site}/channels/{channel}/profiles/{profile['id']}/":
            return checker.FetchText(text=profile_page_text)
        return checker.FetchText(text="", error=f"unexpected text fetch {url}")

    def fake_fetch_json(url: str):
        if url == f"{site}/channels.json":
            return checker.FetchJson(data=channels)
        return checker.FetchJson(data=None, error=f"unexpected json fetch {url}")

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return checker.FetchBytes(b"", f"unexpected fetch {url}")
        return checker.FetchBytes(data)

    def fake_fetch_headers(url: str):
        cache_control = headers.get(url)
        if cache_control is None:
            return checker.FetchHeaders({}, f"unexpected header fetch {url}")
        return checker.FetchHeaders({"cache-control": cache_control})

    checker.fetch_text = fake_fetch_text
    checker.fetch_json = fake_fetch_json
    checker.fetch_bytes = fake_fetch_bytes
    checker.fetch_headers = fake_fetch_headers
    return {
        "site": site,
        "channel": channel,
        "manifest_path": manifest_path,
        "current_binary": current_binary,
        "current_assets": current_assets,
        "profile_revision": profile_revision,
        "manifest": manifest,
        "channels": channels,
    }


def test_remote_readiness_accepts_channels_manifest_profile_graph_contract() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(checker)

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert result.ok, result.detail
    assert "channels.json" in result.detail
    assert "graph manifest" in result.detail
    assert "profile artifacts" in result.detail


def test_remote_readiness_helper_edge_cases_reject_malformed_release_contract() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels.update({"channels": []}),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "channels catalog missing or not an object" in result.detail
    assert "channels.stable missing or not an object" in result.detail


def test_remote_readiness_rejects_stale_index_profile_metadata() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(checker, index_text="1.4.0 2030.0101.1")

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "release index stable missing channel label Stable" in result.detail
    assert (
        "release index stable missing channel description Recommended release channel."
        in result.detail
    )
    assert "release index stable missing manifest URL /assets/stable/manifest.json" in result.detail


def test_remote_readiness_rejects_channel_manifest_digest_drift() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels["channels"]["stable"]["manifests"][0][
            "digest"
        ].update({"blake3": "0" * 64}),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "channel manifest BLAKE3 mismatch" in result.detail


def test_remote_readiness_rejects_manifest_pointer_drift() -> None:
    checker = _readiness_checker_module()
    wrong_manifest_url = "https://release.capsem.org/assets/nightly/manifest.json"

    def copy_manifest_to_wrong_url(payloads: dict[str, bytes], _checker) -> None:
        payloads[wrong_manifest_url] = payloads[
            "https://release.capsem.org/assets/stable/manifest.json"
        ]

    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels["channels"]["stable"]["manifests"][0].update(
            {"url": "/assets/nightly/manifest.json"}
        ),
        payload_mutator=copy_manifest_to_wrong_url,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        "release index stable missing manifest URL /assets/nightly/manifest.json" in result.detail
    )
    assert "channel page stable missing manifest URL /assets/nightly/manifest.json" in result.detail
    assert (
        "unexpected header fetch https://release.capsem.org/assets/nightly/manifest.json"
        in result.detail
    )


def test_remote_readiness_rejects_profile_catalog_artifact_drift() -> None:
    checker = _readiness_checker_module()

    def stale_rootfs_digest(manifest: dict[str, object]) -> None:
        profile = manifest["profiles"]["co-work"]
        architecture = profile["architectures"][0]
        rootfs = next(item for item in architecture["images"] if item["kind"] == "rootfs")
        rootfs["digest"]["blake3"] = "0" * 64

    fixture = _install_release_graph_contract_fixture(
        checker,
        manifest_mutator=stale_rootfs_digest,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        "profile co-work architecture arm64 image "
        "/assets/releases/2030.0101.1/arm64-rootfs.erofs blake3 mismatch" in result.detail
    )


def test_remote_readiness_rejects_profile_catalog_content_drift() -> None:
    checker = _readiness_checker_module()
    source = "/profiles/releases/profiles-2030.0101.1/co-work/arm64/software-inventory.json"

    def stale_inventory(payloads: dict[str, bytes], _checker) -> None:
        payloads[f"https://release.capsem.org{source}"] = (
            b'{"schema":"capsem.profile_software_inventory.v0","packages":[]}\n'
        )

    fixture = _install_release_graph_contract_fixture(checker, payload_mutator=stale_inventory)

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        f"profile co-work architecture arm64 evidence {source} software inventory schema mismatch"
        in result.detail
    )


def test_remote_readiness_rejects_asset_file_metadata_drift() -> None:
    checker = _readiness_checker_module()
    asset_path = "/assets/releases/2030.0101.1/arm64-rootfs.erofs"

    def stale_rootfs_size(manifest: dict[str, object]) -> None:
        profile = manifest["profiles"]["co-work"]
        architecture = profile["architectures"][0]
        rootfs = next(item for item in architecture["images"] if item["kind"] == "rootfs")
        rootfs["bytes"] = 4

    fixture = _install_release_graph_contract_fixture(
        checker,
        manifest_mutator=stale_rootfs_size,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert f"profile co-work architecture arm64 image {asset_path} size mismatch" in result.detail


def test_remote_readiness_rejects_cache_header_drift() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        headers_mutator=lambda headers: headers.update(
            {"https://release.capsem.org/channels.json": "public, max-age=31536000"}
        ),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        "channels JSON https://release.capsem.org/channels.json Cache-Control must contain no-cache"
        in result.detail
    )


def test_binary_release_verifies_packages_hydrate_vm_assets_from_public_channel() -> None:
    verify_downloads = _workflow_job_block("verify-release-downloads", "release.yaml")

    assert "needs: [deploy-release-channel]" in verify_downloads
    assert "scripts/verify-channel-downloads.py" in verify_downloads
    assert '--manifest-url "$ASSET_MANIFEST_URL"' in verify_downloads

    # The checks themselves moved out of the YAML and into a script that tests
    # can call. They were a `curl` loop, a byte comparison and a blake3 check
    # written as a Python heredoc indented inside a `run:` block -- a program no
    # test could reach, guarding the last step before anyone installs a release.
    verifier = _source_text("scripts/verify-channel-downloads.py")
    assert "manifest_asset_rows" in verifier
    assert "m['assets']['current']" not in verifier
    assert "blake3.blake3(payload).hexdigest()" in verifier
    assert "expected_bytes" in verifier
    # The three verdicts the step must still be able to reach, now phrased by
    # the script rather than by a `curl` loop: unreachable, wrong length, wrong
    # bytes. Asserted where they live so they stay callable from a test.
    assert "is not reachable" in verifier
    assert "the manifest declares" in verifier
    assert "hashes to" in verifier
    assert "scripts/check-public-binary-release.py" in verify_downloads
    assert '--channel "$RELEASE_CHANNEL"' in verify_downloads
    assert (
        "--stable-manifest-url https://release.capsem.org/assets/stable/manifest.json"
    ) in verify_downloads
    assert (
        "--nightly-manifest-url https://release.capsem.org/assets/nightly/manifest.json"
    ) in verify_downloads
    assert '--manifest-url "$ASSET_MANIFEST_URL"' in verify_downloads
    assert "--install-script-url https://capsem.org/install.sh" in verify_downloads
    assert "--docker-linux-install" not in verify_downloads
    assert "Enable KVM for live public-install VM proof" in verify_downloads
    assert "Install live public Linux release and prove guest shell execution" in verify_downloads
    assert "scripts/prove-live-public-install.sh" in verify_downloads
    live_proof = (PROJECT_ROOT / "scripts/prove-live-public-install.sh").read_text()
    assert 'curl -fsSL https://capsem.org/install.sh | CAPSEM_CHANNEL="$channel" sh' in live_proof
    assert "dpkg-query -W -f='${Version}' capsem" in live_proof
    assert 'grep -F "Running:   true" /tmp/capsem-live-status.txt' in live_proof
    assert 'grep -F "Service:   ok" /tmp/capsem-live-status.txt' in live_proof
    assert 'grep -F "Gateway:   ok" /tmp/capsem-live-status.txt' in live_proof
    assert '"$script_dir/prove-installed-shell.py"' in live_proof
    assert '"$script_dir/verify-installed-release.py"' in live_proof
    assert "CAPSEM_LIVE_PUBLIC_INSTALL_SHELL_OK" in live_proof
    assert '"$HOME/.capsem/bin/capsem" run' not in verify_downloads
    assert "skipping binary e2e" not in verify_downloads
    assert "::warning::no .deb" not in verify_downloads
    assert "::warning::no 'capsem' CLI" not in verify_downloads


def test_manifest_source_inputs_are_url_only() -> None:
    build_pkg = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "build-pkg.sh").read_text()
    repack_deb = (
        PROJECT_ROOT / "build_system/packaging/linux/repack-deb.sh"
    ).read_text()
    release = _workflow_text("release.yaml")
    release_assets = _workflow_text("release-assets.yaml")
    release_channel = _workflow_text("release-channel.yaml")
    # Production rejection message plus its unit tests, which live in the
    # sibling tests.rs; the assertions below span both.
    admin = (PROJECT_ROOT / "crates/capsem-admin/src/main.rs").read_text() + (
        PROJECT_ROOT / "crates/capsem-admin/src/tests.rs"
    ).read_text()

    for script in (build_pkg, repack_deb):
        assert "--manifest requires a URL" in script
        assert "manifest must be a URL" in script
        assert "pathlib.Path(source).read_bytes()" not in script

    workflows = {
        "release.yaml": release,
        "release-assets.yaml": release_assets,
        "release-channel.yaml": release_channel,
    }
    package_commands = []
    for name, workflow in workflows.items():
        document = yaml.safe_load(workflow)
        for job_name, job in document["jobs"].items():
            for step in job.get("steps", ()):
                shell = step.get("run") if isinstance(step, dict) else None
                if not isinstance(shell, str):
                    continue
                package_commands.extend(
                    command
                    for command in parsed_commands(shell, origin=f"{name}:{job_name}")
                    if {
                        "build_system/packaging/macos/build-pkg.sh",
                        "build_system/packaging/linux/repack-deb.sh",
                    }.intersection(command.argv)
                )

    assert package_commands
    for command in package_commands:
        manifest = command.argv[command.argv.index("--manifest") + 1]
        if manifest == "$ASSET_MANIFEST_URL":
            assert "ASSET_MANIFEST_URL: https://release.capsem.org/assets/" in release
        else:
            assert manifest.startswith(("file://", "https://", "http://"))

    assert "manifest must be a URL" in admin
    assert "unsupported {label} URL scheme" in admin


def test_asset_channel_documented_as_assets_manifest_url_not_release_index_json() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    asset_skill = _skill_text("skills/asset-pipeline/SKILL.md")
    release_skill = _skill_text("skills/release-process/SKILL.md")
    release_skill_text = " ".join(release_skill.split())

    for text in (docs,):
        normalized_text = " ".join(text.split())
        assert "https://release.capsem.org/assets/stable/manifest.json" in text
        assert "target/distribution/assets/<channel>/manifest.json" in text or (
            "target/distribution/assets/stable/manifest.json" in text
        )
        assert "https://release.capsem.org/assets/nightly/manifest.json" in text
        assert "https://release.capsem.org/channels.json" in text
        assert "`channels.json`" in text
        assert "host SBOM" in text
        assert "package artifacts" in text
        assert "per-binary inventory" in text
        assert "versioned manifest records" in text
        assert "`current`, `supported`, `deprecated`, or `revoked`" in normalized_text
        assert "`min_capsem_version`" in text
        assert "first channel bootstrap may have no host binary evidence yet" in normalized_text
        assert (
            "once binary files are published, missing host SBOM evidence is release-blocking"
            in normalized_text
        )
        assert "stable-to-nightly acceptance gate" in normalized_text
        assert "channels/stable/index.json" not in text

    asset_skill_text = " ".join(asset_skill.split())
    assert "https://release.capsem.org/assets/stable/manifest.json" in asset_skill
    assert "target/distribution/assets/<channel>/manifest.json" in asset_skill
    assert "`channels.json`" in asset_skill
    assert "package artifacts separate from per-binary inventory" in asset_skill_text
    assert (
        "Profiles own profile images, config files, software inventory, ABOM/OBOM evidence"
        in asset_skill_text
    )
    assert "channels/stable/index.json" not in asset_skill

    assert "https://release.capsem.org/assets/stable/manifest.json" in release_skill
    assert "target/distribution/assets/<channel>/manifest.json" in release_skill
    assert "`channels.json`" in release_skill
    assert "Profiles belong to channels" in release_skill
    assert "Packages are delivery containers" in release_skill_text
    assert "binary inventory is nested under it" in release_skill_text.lower()
    assert (
        "owns its config, images, software inventory, obom/evidence"
        in release_skill_text.lower()
    )
    assert "channels/stable/index.json" not in release_skill


def test_release_skill_keeps_binary_and_asset_verification_decoupled() -> None:
    release_skill = _skill_text("skills/release-process/SKILL.md")
    release_skill_text = " ".join(release_skill.split())

    assert "`just release-binaries <channel> <source-commit>`" in release_skill
    assert "`just release-profile <channel> <profile> <source-commit>`" in release_skill
    assert "binary lane builds packages only" in release_skill_text
    assert "profile lane builds exactly one channel/profile" in release_skill_text
    assert "Neither artifact family is rebuilt twice" in release_skill
    assert (
        "selected channel source manifest is the sole mutable release authority"
        in release_skill_text
    )
    assert "`release-channel.yaml` may deploy production only" in release_skill_text
    assert "scripts/check-release-site-contract.py" in release_skill


def test_release_process_skill_documents_multi_channel_graph() -> None:
    release_skill = _skill_text("skills/release-process/SKILL.md")
    release_skill_text = " ".join(release_skill.split())

    for required in [
        "Profiles belong to channels",
        "a profile may exist in stable and nightly independently",
        "a profile may exist only in nightly",
        "`current`, `supported`, `deprecated`, or `revoked`",
        "Packages are delivery containers",
        "binary inventory is nested under it",
        "minimum compatible Capsem version",
        "`https://release.capsem.org/assets/stable/manifest.json`",
        "`https://release.capsem.org/assets/nightly/manifest.json`",
        "`release-channel.yaml` deploys a generated distribution",
        "Dependent profile then binary",
    ]:
        assert required.lower() in release_skill_text.lower(), required

    assert "Binary lane" in release_skill
    assert "Profile lane" in release_skill
    assert "Corporate authoring" in release_skill
    assert (
        "same revision label in two channels cannot alias or overwrite bytes" in release_skill_text
    )
    assert "schema_version" not in release_skill


def test_docs_describe_multi_channel_release_graph() -> None:
    docs_paths = [
        PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md",
        PROJECT_ROOT / "docs/src/content/docs/development/ci.md",
        PROJECT_ROOT / "docs/src/content/docs/architecture/build-system.md",
    ]
    combined = "\n".join(path.read_text() for path in docs_paths)
    combined_text = " ".join(combined.split())

    for required in [
        "`channels.json`",
        "stable and nightly",
        "versioned manifest records",
        "exactly one `status` enum value",
        "`current`, `supported`, `deprecated`, or `revoked`",
        "package artifacts",
        "per-binary inventory",
        "Every executable inside each package must be listed",
        "SHA-256, and BLAKE3",
        "HMAC fields are not published",
        "`min_capsem_version`",
        "Profiles own profile images, config files, software inventory, and ABOM/OBOM",
        "profile-owned config, image, ABOM, and OBOM files",
        "https://release.capsem.org/assets/stable/manifest.json",
        "https://release.capsem.org/assets/nightly/manifest.json",
        "stable-to-nightly acceptance gate",
        "absence from the channel list",
        "--manifest file:///path/to/assets/manifest.json",
    ]:
        assert required in combined_text, required

    assert "health.json" not in combined
    assert "capsem.assets_channel.health.v1" not in combined
    assert "current binary" not in combined_text
    assert "VM artifact" not in combined
    assert "schema_version" not in combined


def test_asset_and_install_skills_document_channel_switching() -> None:
    asset_skill = _skill_text("skills/asset-pipeline/SKILL.md")
    install_skill = (PROJECT_ROOT / "skills/dev-installation/SKILL.md").read_text()
    combined = "\n".join([asset_skill, install_skill])
    combined_text = " ".join(combined.split())

    for required in [
        "`channels.json` lists all channels",
        "all versioned manifest records",
        "one status enum value",
        "`current`, `supported`, `deprecated`, or `revoked`",
        "package artifacts separate from per-binary inventory",
        "Profiles own profile images, config files, software inventory, ABOM/OBOM evidence",
        "`min_capsem_version`",
        "`--manifest` must be a URL",
        "`--manifest` and `--corp` are URL-only inputs",
        "`file:///absolute/path/to/manifest.json`",
        "`https://release.capsem.org/assets/stable/manifest.json`",
        "`https://release.capsem.org/assets/nightly/manifest.json`",
        "single metadata file records the installed manifest URL separately",
        "Updating the co-work nightly profile",
        "must not mutate stable, packages, per-binary inventory, or other profiles",
    ]:
        assert required in combined_text, required

    assert "health.json" not in combined
    assert "current binary" not in combined_text
    assert "VM artifact" not in combined
    assert "schema_version" not in combined


def test_release_skills_preserve_vm_obom_attestation_predicate_contract() -> None:
    release_skill = _skill_text("skills/release-process/SKILL.md")
    asset_skill = _skill_text("skills/asset-pipeline/SKILL.md")

    for skill in (release_skill, asset_skill):
        skill_text = " ".join(skill.split())
        assert "VM asset attestations are incomplete unless" in skill_text
        assert "`github_attestations_vm_assets`" in skill_text
        assert "`predicate_url` points at the published VM OBOM evidence" in skill_text


def test_site_skills_preserve_every_main_merge_deploy_rail() -> None:
    site_infra_skill = (PROJECT_ROOT / "skills/site-infra/SKILL.md").read_text()
    site_marketing_skill = (PROJECT_ROOT / "skills/site-marketing/SKILL.md").read_text()
    site_infra_text = " ".join(site_infra_skill.split())
    site_marketing_text = " ".join(site_marketing_skill.split())

    assert "`ci.yaml` runs the merge-blocking `docs-build` job" in site_infra_text
    assert (
        "deploys only on every push to `main` and smokes `https://docs.capsem.org/`"
        in site_infra_text
    )
    assert "requires the warmed `/getting-started/` tombstone markers" in site_infra_text
    assert "`ci.yaml` runs the merge-blocking `site-build` job" in site_marketing_text
    assert (
        "deploys only on every push to `main` and smokes `https://capsem.org/`"
        in site_marketing_text
    )

    for skill in (site_infra_skill, site_marketing_skill):
        assert "independent from binary releases" in skill
        assert "manual VM asset releases" in skill
        assert "`release.capsem.org` asset-channel workflow" in skill


def test_capsem_update_checks_release_channel_manifest_not_github_latest() -> None:
    update_rs = (PROJECT_ROOT / "crates/capsem/src/update.rs").read_text()

    assert "https://release.capsem.org/assets/stable/manifest.json" in update_rs
    assert "DEFAULT_RELEASE_MANIFEST_URL" in update_rs
    assert "CAPSEM_RELEASE_MANIFEST_URL" in update_rs
    assert "api.github.com/repos/google/capsem/releases/latest" not in update_rs


def test_docs_do_not_teach_bare_manifest_paths_for_package_inputs() -> None:
    docs = [
        PROJECT_ROOT / "docs/src/content/docs/architecture/asset-pipeline.md",
        PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md",
    ]

    for path in docs:
        text = path.read_text()
        assert "--manifest /path/to/assets/manifest.json" not in text, path
        assert "--manifest file:///path/to/assets/manifest.json" in text, path


def test_asset_skill_documents_custom_manifest_url_contract() -> None:
    skill = _skill_text("skills/asset-pipeline/SKILL.md")

    assert "capsem update --assets --manifest <URL>" in skill
    assert "`--manifest` is URL-shaped" in skill
    assert "`file:///absolute/path/to/manifest.json`" in skill
    assert "`https://...` or `http://...`" in skill
    assert "`--corp` provisions corporate policy config" in skill


def test_ci_docs_describes_three_independent_publication_rails() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    normalized_docs = " ".join(docs.split())

    assert (
        "| `release-nightly.yaml` | Daily schedule or manual dispatch | Freeze `${{ github.sha }}`, then dispatch both profile commands and the binary command; each hosted lane qualifies the exact artifacts it may publish |"
        in docs
    )
    assert (
        "| `release.yaml` | Correlated dispatch from `release-binaries` with `{tag, channel, publish, dispatch_id, source_commit}` | Build and install-test exact native packages from the selected immutable commit; publish and advance only a new immutable identity, or finish as a rebuild-only proof when that identity already exists |"
        in docs
    )
    assert (
        "| `release-assets.yaml` | Correlated dispatch from `capsem-admin release` with `source_commit` | Build exactly one channel/profile's images, config, and evidence from that commit against the existing channel package; the public command watches that exact run through success |"
        in docs
    )
    assert (
        "| `release-channel-staging.yaml` | Manual | Build a deterministic staging asset channel fixture, deploy it to a Cloudflare Pages preview branch, and validate the same release-channel contract without invoking `build-assets`, `build-app-macos`, or `build-app-linux` |"
        in docs
    )
    assert (
        "| `release-binary-staging.yaml` | Manual | Build a deterministic binary-channel dry-run bundle from fake host packages and the live asset manifest, then prove profile image metadata is unchanged without creating a GitHub release or deploying release.capsem.org |"
        in docs
    )
    assert (
        "| `docs.yaml` | Push to main when docs or a shared docs-build input changes | Deploy docs.capsem.org, then smoke the live docs site |"
        in docs
    )
    assert (
        "| `site.yaml` | Push to main when marketing, graphics, or a shared site-build input changes | Deploy capsem.org, then smoke the live marketing site |"
        in docs
    )
    assert (
        "| `release-channel.yaml` | Called by binary or asset release | Validate the generated distribution on an immutable preview, activate it on release.capsem.org, and restore the prior production deployment on any activation-verification failure |"
        in docs
    )
    assert "release.yaml` | Tag push (`v*`) | Build assets" not in docs
    assert "generated asset manifest artifact" not in docs
    assert "### pr-gate (ubuntu-latest)" in docs
    assert "`scope`, `fast-gate`, `test-linux`, `test`, `test-install`, `docs-build`," in docs
    assert "`site-build`, and `release-site-build`, and runs even" in docs
    assert "must report `success` when selected and `skipped` when not" in normalized_docs
    assert "After Cloudflare activates production, `release-channel.yaml` checks" in normalized_docs
    assert "`https://release.capsem.org/` index" in docs
    assert "`/channels.json`, and" in docs
    assert "`/assets/<channel>/manifest.json` before the workflow can pass" in normalized_docs
    assert (
        "`docs.yaml` and `site.yaml` are independent from binary and profile image release" in docs
    )
    assert "`https://docs.capsem.org/`, content type `text/html`" in docs
    assert "`https://capsem.org/`, content type `text/html`" in docs


def test_ci_docs_compare_pr_gate_to_just_test_with_named_substitutions() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    for stage in [
        "Audits + lint + web surfaces",
        "Cross-compile agent (both arches)",
        "Rust: test suite with coverage",
        "Python: non-serial tests (n=4 parallel)",
        "Python: serial timing and benchmark tests",
        "Fast source and serialized release contracts",
        "Injection test",
        "Integration test",
        "Benchmarks",
        "Cross-compile Linux releases (Docker, both arches)",
        "Install e2e tests (Docker + systemd)",
    ]:
        marker = _GATE_STAGES[stage]
        assert any(label.startswith(marker) for label in _gate_labels()), (
            f"{stage!r} is documented as a gate stage, but no step matching "
            f"{marker!r} exists in the gate"
        )

    assert "## PR gate compared with `just test-clean`" in docs
    assert (
        "| YAML/source syntax, source contracts, audits, lint, and all web surfaces | `fast-gate` calls the same `_test-fast` module used first by `just test-clean` and run alone by `just fast-test`; dedicated web jobs retain platform/deployment evidence | One independently executable fast module, including blocking vulnerability audits across all locked ecosystems |"
        in docs
    )
    assert (
        "| VM-heavy Python suites (`pytest tests/ -n 4`) | Import collection only on hosted PR runners | Runner substitution: full execution remains a local/release gate until PR runners can host Apple VZ reliably |"
        in docs
    )
    assert (
        "| Legacy injection/integration scripts and benchmark recording | Not run in hosted PR CI | Run through the owning focus group during diagnosis and by hosted release qualification before publication |"
        in docs
    )
    assert (
        "| Docs, marketing, and release-channel site builds | When selected by the fail-closed path owner, `docs-build`, `site-build`, and `release-site-build` call the same web-surface entrypoint as `just test-clean` before `pr-gate` can pass | Owner-scoped duplicate execution of the canonical local gate; deploy happens only after an owned or shared merge, or explicit release-channel publication |"
        in docs
    )
    assert "`pr-gate` is the only status that should be required by branch protection" in docs
    assert "`pr-gate` depends on `docs-build`, `site-build`, and `release-site-build`" in docs_text
    assert frozenset(_workflow_job("pr-gate")["needs"]) == REQUIRED_PR_GATE_JOBS


def test_release_skills_require_local_ci_execution_parity_and_record_native_musl_lesson() -> None:
    testing = _skill_text("skills/dev-testing/SKILL.md")
    skills = (PROJECT_ROOT / "skills/dev-skills/SKILL.md").read_text()
    debugging = (PROJECT_ROOT / "skills/dev-debugging/SKILL.md").read_text()
    release = _skill_text("skills/release-process/SKILL.md")

    for document in (testing, debugging, release):
        assert "Local/CI execution parity" in document
        assert "same production entrypoint" in document
        assert "Docker" in document

    assert "local/CI parity" in skills
    assert "native `musl-gcc`" in skills
    assert "`x86_64-linux-musl-gcc`" in skills
    assert "unavoidable platform boundary" in testing
    assert "Apple VZ is proven by the complete local gate" in testing
    assert "Release CI reuses the same checked-in private modules" in testing
    assert "release-assets.yaml" in release
    assert "linux_musl_toolchain_available" in release


def test_release_critical_workflows_share_local_entrypoints_or_name_platform_boundaries() -> None:
    just = (PROJECT_ROOT / "justfile").read_text()
    macos_glowup = _source_text("build_system/packaging/macos/macos_release_glowup.py")
    assets = _workflow_text("release-assets.yaml")
    ci = _workflow_text("ci.yaml")
    release = _workflow_text("release.yaml")
    fast_gate = _workflow_text("fast-gate.yaml")
    release_skill = _skill_text("skills/release-process/SKILL.md")

    assert "test-clean" in just
    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate
    assert "uses: ./.github/workflows/fast-gate.yaml" in assets
    assert "uses: ./.github/workflows/fast-gate.yaml" in release
    assert "just qualify-assets" in assets
    assert "just qualify-binaries" in release
    assert "just _test-release-contracts" not in assets
    assert "just _test-release-contracts" not in release

    assert "just build-assets" in assets
    assert "build-assets arch" in just

    assert "uv run --project build_system --frozen capsem-gate install" in ci
    assert "_gate-install:" in just
    install_job = ci.split("  test-install:", 1)[1].split("\n  #", 1)[0]
    assert "runs-on: ubuntu-24.04" in install_job
    assert "sudo modprobe kvm" in install_job
    assert "sudo modprobe vhost_vsock" in install_job
    assert "test -r /dev/kvm -a -w /dev/kvm" in install_job
    assert "test -r /dev/vhost-vsock -a -w /dev/vhost-vsock" in install_job

    for shared_script in (
        "build_system/packaging/macos/build-pkg.sh",
        "scripts/verify-installed-release.py",
        "scripts/prove-installed-shell.py",
    ):
        assert shared_script in release
    assert "uv run --project build_system --frozen capsem-gate cross-compile" in release
    assert '"$SCRIPT_DIR/repack-deb.sh"' in _source_text(
        "build_system/packaging/linux/build-linux-package.sh"
    )
    assert "config.install.local_macos_package_script" in macos_glowup
    # The local rail must reach the same shared scripts CI does. Since the
    # package build moved out of the recipe body, "local" is the justfile plus
    # what it dispatches to -- the point of the rule is that both rails run the
    # same code, not that one file contains it.
    # "Local" is the justfile plus everything it dispatches to: the gate
    # package, the config that names the scripts it runs, and the build script
    # the package rail hands to its builder. The point of the rule is that both
    # rails execute the same code, not that one file contains it.
    local_rail = "\n".join(
        [
            just,
            (
                PROJECT_ROOT / "build_system/packaging/linux/build-linux-package.sh"
            ).read_text(),
            (PROJECT_ROOT / "config" / "gate.toml").read_text(),
            *(
                path.read_text()
                for path in sorted((PROJECT_ROOT / "build_system" / "builder" / "gate").glob("*.py"))
            ),
        ]
    )
    for shared_script in (
        '"$SCRIPT_DIR/repack-deb.sh"',
        "scripts/verify-installed-release.py",
        "scripts/prove-installed-shell.py",
    ):
        assert shared_script in local_rail

    for unavoidable_boundary in (
        "Apple signing and notarization",
        "hosted-runner KVM",
        "Cloudflare publication",
    ):
        assert unavoidable_boundary in release_skill
    assert "Apple VZ is owned by the complete" in release_skill
    assert "Local Apple Silicon `just test-clean` owns that VZ proof" in release_skill


def test_web_surfaces_share_one_local_and_ci_entrypoint() -> None:
    script = _source_text("scripts/check-web-surface.sh")
    shell = parse_shell(script)
    just = (PROJECT_ROOT / "justfile").read_text()
    ci = _workflow_text("ci.yaml")
    docs = _workflow_text("docs.yaml")
    site = _workflow_text("site.yaml")
    release = _workflow_text("release.yaml")
    release_assets = _workflow_text("release-assets.yaml")
    binary_staging = _workflow_text("release-binary-staging.yaml")
    channel_staging = _workflow_text("release-channel-staging.yaml")
    binary_staging_builder = _source_text("scripts/build-complete-release-channel.py")
    channel_staging_rehearsal = _source_text("scripts/rehearse-asset-channel-staging.sh")

    for surface in (
        "frontend-verify",
        "frontend-build",
        "docs",
        "site",
        "release-site",
        "release-site-build",
    ):
        assert arm_named(shell, surface) is not None

    fast = _dispatched_text("test-clean:")
    for surface in ("frontend-verify", "docs", "site", "release-site"):
        assert f"check-web-surface.sh {surface}" in fast

    assert "bash scripts/check-web-surface.sh frontend-verify" in ci
    assert "bash scripts/check-web-surface.sh docs" in ci
    assert "bash scripts/check-web-surface.sh site" in ci
    assert "bash scripts/check-web-surface.sh release-site" in ci
    assert "bash scripts/check-web-surface.sh docs" in docs
    assert "bash scripts/check-web-surface.sh site" in site
    assert release.count("bash scripts/check-web-surface.sh frontend-build") == 1
    assert "bash scripts/check-web-surface.sh frontend-build" in _source_text(
        "build_system/packaging/linux/build-linux-package.sh"
    )
    assert "scripts/build-complete-release-channel.py" in binary_staging
    assert '"scripts/check-web-surface.sh", "release-site-build"' in binary_staging_builder
    assert "bash scripts/rehearse-asset-channel-staging.sh" in channel_staging
    assert "bash scripts/check-web-surface.sh release-site-build" in channel_staging_rehearsal
    assert "scripts/build-complete-release-channel.py" in release
    assert "scripts/build-complete-release-channel.py" in release_assets

    bypasses = (
        "cd web/app && pnpm run build",
        "cd web/app && pnpm build",
        "cd docs && pnpm run build",
        "cd site && pnpm run build",
        "cd build_system/release_site && pnpm run build:channel",
    )
    for text in (
        just,
        ci,
        docs,
        site,
        release,
        release_assets,
        binary_staging,
        channel_staging,
        binary_staging_builder,
        channel_staging_rehearsal,
    ):
        for bypass in bypasses:
            assert bypass not in text

    assert "write-release-site-ci-fixture.py" in script
    assert "build-complete-release-channel.py" in script
    assert "pnpm --dir build_system/release_site run build:channel" in script
    assert 'test -s "$CAPSEM_RELEASE_CHANNEL_DIST/404.html"' in script
    assert 'grep -q "Artifact not found"' in script
    complete_builder = _source_text("scripts/build-complete-release-channel.py")
    assert '"assets",\n                "channel",\n                "check"' in complete_builder
    assert "CAPSEM_FRONTEND_JUNIT" in script


def test_release_channel_fixture_keeps_obom_evidence_architecture_owned(tmp_path: Path) -> None:
    """The fixture must model the architecture identity of real OBOM evidence."""
    fixture = tmp_path / "fixture"
    subprocess.run(
        [sys.executable, "scripts/write-release-site-ci-fixture.py", str(fixture)],
        cwd=PROJECT_ROOT,
        check=True,
    )
    documents = {
        arch: (fixture / "assets" / arch / "obom.cdx.json").read_bytes()
        for arch in ("arm64", "x86_64")
    }

    assert len(set(documents.values())) == len(documents), (
        "each architecture must own distinct OBOM evidence bytes"
    )
    for arch, payload in documents.items():
        document = json.loads(payload)
        component = document["metadata"]["component"]
        assert component["name"] == f"capsem-rootfs-{arch}"
        assert {(item["name"], item["value"]) for item in component["properties"]} >= {
            ("capsem:evidence:scope", "exported-rootfs"),
            ("capsem:guest:architecture", arch),
        }


def test_ironbank_release_rule_is_the_complete_local_and_ci_just_test() -> None:
    binary = _workflow_text("release.yaml")
    profile = _workflow_text("release-assets.yaml")
    fast_gate = _workflow_text("fast-gate.yaml")
    testing = _skill_text("skills/dev-testing/SKILL.md")
    ironbank = _skill_text("skills/ironbank/SKILL.md")
    release = _skill_text("skills/release-process/SKILL.md")

    for document in (testing, ironbank, release):
        assert "Ironbank parity rule" in document
        assert "every portable release gate" in document
        assert "`just test-clean`" in document

    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate
    for workflow in (binary, profile):
        assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
        assert "just qualify-binaries" in workflow or "just qualify-assets" in workflow
        assert "just _test-release-contracts" not in workflow
    gate = _dispatched_text("test-clean:")
    assert "cargo llvm-cov" in gate
    assert RUST_LINE_COVERAGE_FLOOR.replace(" ", "=") in gate
    # The Python floor is `fail_under` in pyproject's [tool.coverage.report], so
    # every run that reports inherits it. What the gate must still do is measure:
    # a run with no `--cov` reports nothing, and a floor over nothing passes.
    assert "--cov" in gate, "the Python suite runs without measuring coverage"
    assert "CAPSEM_REQUIRE_ARTIFACTS=1" in gate
    assert "tests/ironbank/test_route_health.py" in gate
    assert "integration_test.py" in gate
    assert "tests/capsem-serial/test_capsem_bench_baseline.py" in gate
    assert "install the exact package" in gate
    for surface in ("frontend", "docs", "site", "release-site"):
        assert f"check-web-surface.sh {surface}" in gate


def test_release_channel_deploy_validates_the_deployed_channel_shape() -> None:
    deploy = _workflow_text("release-channel.yaml")
    staging = _workflow_text("release-channel-staging.yaml")

    assert "validate_complete_public_channels:" in deploy
    assert 'CHANNEL_ARGS=(--channel "$CHANNEL")' in deploy
    assert "CHANNEL_ARGS=(--catalog-members)" in deploy
    assert "CHANNEL_ARGS=(--channel stable --channel nightly)" not in deploy
    assert '"${CHANNEL_ARGS[@]}"' in deploy
    assert "validate_complete_public_channels: false" in staging
    assert "activate_production: false" in staging


def test_remote_release_readiness_checker_is_read_only_and_covers_live_gates() -> None:
    script = _source_text("scripts/check-remote-release-readiness.py")
    remote_gate = _source_text("build_system/builder/release/tools/remote_ci_gate.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "Read-only remote release readiness checks" in script
    assert 'git", "rev-list", "--left-right", "--count"' in script
    assert 'gh", "workflow", "view", "ci.yaml"' in script
    assert "application/vnd.github.raw+json" in script
    assert "branches/{branch}/protection" in script
    assert "repos/{repo}/rules/branches/{branch}" in script
    assert "socket.getaddrinfo" in script
    assert "urllib.request.urlopen" in script
    assert "https://release.capsem.org" in script
    assert "/assets/{channel}/manifest.json" in script
    assert "/channels.json" in script
    assert "channels catalog" in script
    assert "channel manifest BLAKE3 mismatch" in script
    assert "pr-gate" in script
    assert "REQUIRED_PR_GATE_JOBS" in remote_gate
    assert "gate_script_contract_failures" in remote_gate
    assert '"release-site-build"' in remote_gate
    assert "current asset release date" in script
    assert 'RELEASE_VALIDATOR_USER_AGENT = "CapsemReleaseValidator/1.0"' in script
    assert "release_site_request(url)" in script
    for forbidden in [
        "git push",
        "gh release create",
        "gh release upload",
        "wrangler",
        "pages deploy",
        "--method PATCH",
        "--method PUT",
        "--method DELETE",
    ]:
        assert forbidden not in script

    assert "scripts/check-remote-release-readiness.py" in docs
    assert "read-only" in docs
    assert "reads both `ci.yaml` and its dispatched verdict script from that commit" in docs_text
    assert (
        "aggregates `scope`, `fast-gate`, `test-linux`, `test`, `test-install`, `docs-build`, `site-build`, and `release-site-build`"
        in (docs_text)
    )
    assert "runs with `if: ${{ always() }}`" in docs_text
    assert "rejects every failing, cancelled, or unexpectedly skipped dependency result" in docs_text
    assert "branch protection or active branch rulesets require `pr-gate`" in docs_text
    assert "`release.capsem.org` resolves and serves the generated release graph" in docs_text


def test_remote_release_readiness_fetches_with_validator_user_agent(monkeypatch) -> None:
    checker = _readiness_checker_module()
    requests = []

    class FakeResponse:
        headers: ClassVar[dict[str, str]] = {"Cache-Control": "no-cache, must-revalidate"}

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def read(self) -> bytes:
            return b"ok"

    def fake_urlopen(request, *, timeout: int):
        requests.append(request)
        assert timeout == 20
        return FakeResponse()

    monkeypatch.setattr(checker.urllib.request, "urlopen", fake_urlopen)

    body = checker.fetch_bytes("https://release.capsem.org/")
    headers = checker.fetch_headers("https://release.capsem.org/health.json")

    assert body == checker.FetchBytes(b"ok")
    assert headers == checker.FetchHeaders({"cache-control": "no-cache, must-revalidate"})
    assert [request.full_url for request in requests] == [
        "https://release.capsem.org/",
        "https://release.capsem.org/health.json",
    ]
    assert requests[0].get_header("User-agent") == "CapsemReleaseValidator/1.0"
    assert requests[1].get_header("User-agent") == "CapsemReleaseValidator/1.0"
    assert requests[1].get_method() == "HEAD"


def test_remote_release_readiness_fetch_retries_ipv4_on_network_unreachable(monkeypatch) -> None:
    checker = _readiness_checker_module()
    calls: list[tuple[str, str]] = []
    failures_left = {
        ("GET", "https://release.capsem.org/ipv6-body"): 1,
        ("HEAD", "https://release.capsem.org/ipv6-headers"): 1,
    }

    class FakeResponse:
        headers: ClassVar[dict[str, str]] = {"Cache-Control": "no-cache, must-revalidate"}

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def read(self) -> bytes:
            return b"ok"

    def fake_urlopen(request, *, timeout: int):
        method = request.get_method()
        key = (method, request.full_url)
        calls.append(key)
        assert timeout == 20
        if failures_left.get(key, 0) > 0:
            failures_left[key] -= 1
            raise checker.urllib.error.URLError(
                OSError(checker.errno.ENETUNREACH, "Network is unreachable")
            )
        return FakeResponse()

    monkeypatch.setattr(checker.urllib.request, "urlopen", fake_urlopen)

    body = checker.fetch_bytes("https://release.capsem.org/ipv6-body")
    headers = checker.fetch_headers("https://release.capsem.org/ipv6-headers")

    assert body == checker.FetchBytes(b"ok")
    assert headers == checker.FetchHeaders({"cache-control": "no-cache, must-revalidate"})
    assert calls.count(("GET", "https://release.capsem.org/ipv6-body")) == 2
    assert calls.count(("HEAD", "https://release.capsem.org/ipv6-headers")) == 2


def test_dependent_release_activation_order_is_documented() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = _skill_text("skills/release-process/SKILL.md")

    for text in (docs, release_skill):
        normalized = " ".join(text.split())
        assert "release-profile" in normalized
        assert "release-binaries" in normalized
        assert "capsem-release-" in normalized
        assert "profile" in normalized.lower()
        assert "binary" in normalized.lower()
        assert "without rebuilding" in normalized.lower() or "rebuilt twice" in normalized.lower()
        assert "complete" in normalized.lower()
        assert "glow-up" in normalized.lower()


def test_remote_release_readiness_requires_expanded_pr_gate() -> None:
    module = REMOTE_CI_GATE

    inline = """
jobs:
  scope:
    runs-on: ubuntu-latest
  test-linux:
    runs-on: ubuntu-latest
  test:
    runs-on: ubuntu-latest
  test-install:
    runs-on: ubuntu-latest
  docs-build:
    runs-on: ubuntu-latest
  site-build:
    runs-on: ubuntu-latest
  release-site-build:
    runs-on: ubuntu-latest
  fast-gate:
    uses: ./.github/workflows/fast-gate.yaml
  pr-gate:
    needs: [scope, fast-gate, test-linux, test, test-install, docs-build, site-build, release-site-build]
""".strip()
    multiline = """
jobs:
  pr-gate:
    needs:
      - fast-gate
      - scope
      - test-linux
      - test
      - test-install
      - docs-build
      - site-build
      - release-site-build
    if: ${{ always() }}
""".strip()
    stale = inline.replace(", docs-build, site-build, release-site-build", "")
    assert module.workflow_job_needs(module.workflow_job_block(inline, "pr-gate")) == {
        "scope",
        "fast-gate",
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
    }
    assert module.workflow_job_needs(module.workflow_job_block(multiline, "pr-gate")) == {
        "scope",
        "fast-gate",
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
    }
    assert not {
        "docs-build",
        "site-build",
        "release-site-build",
    }.issubset(module.workflow_job_needs(module.workflow_job_block(stale, "pr-gate")))
    current_gate = module.workflow_job_block(_workflow_text("ci.yaml"), "pr-gate")
    verdict = _source_text("build_system/scripts/ci/require-ci-jobs.sh")
    assert module.pr_gate_contract_failures(current_gate, verdict) == []
    assert module.pr_gate_contract_failures(
        module.workflow_job_block(stale, "pr-gate"), verdict
    )


def test_remote_release_readiness_checker_reports_unpublished_local_commits() -> None:
    script = _source_text("scripts/check-remote-release-readiness.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "def check_local_branch_publication" in script
    assert "HEAD is ahead of {base} by {ahead} commit(s)" in script
    assert "HEAD is behind {base} by {behind} commit(s)" in script
    assert "publish or merge release-rail commits before claiming remote readiness" in script
    assert "local checkout has unpublished commits" in docs_text
    assert "publish or merge those commits before changing remote protection" in docs_text


def test_remote_release_readiness_missing_dependency_reports_setup_hint(tmp_path: Path) -> None:
    shadow = tmp_path / "shadow"
    shadow.mkdir()
    (shadow / "blake3.py").write_text(
        "raise ModuleNotFoundError(\"No module named 'blake3'\")\n",
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable,
            str(PROJECT_ROOT / "scripts/check-remote-release-readiness.py"),
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=30,
        env={"PYTHONPATH": str(shadow)},
    )

    assert result.returncode == 2
    assert "missing Python dependency: blake3" in result.stderr
    assert "uv run --project build_system --frozen python scripts/check-remote-release-readiness.py" in result.stderr
    assert "Traceback" not in result.stderr


def test_remote_release_readiness_requires_active_pr_gate_rule() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert module.classic_protection_requires_pr_gate(
        {"required_status_checks": {"contexts": ["pr-gate"]}}
    )
    assert module.classic_protection_requires_pr_gate(
        {"required_status_checks": {"checks": [{"context": "pr-gate"}]}}
    )
    assert module.active_branch_rules_require_pr_gate(
        [
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": [
                        {"context": "test-linux"},
                        {"context": "pr-gate"},
                    ]
                },
            }
        ]
    )
    assert not module.active_branch_rules_require_pr_gate(
        {
            "enforcement": "evaluate",
            "rules": [
                {
                    "type": "required_status_checks",
                    "parameters": {"required_status_checks": [{"context": "pr-gate"}]},
                }
            ],
        }
    )
    assert not module.active_branch_rules_require_pr_gate(
        [{"type": "pull_request", "parameters": {"message": "mention pr-gate only"}}]
    )
    assert "repos/{repo}/rules/branches/{branch}" in script
    assert "repos/{repo}/rulesets/{ruleset_id}" not in script
    assert "active branch rules" in script
    assert "branch protection or active branch rulesets require `pr-gate`" in docs_text


def test_remote_release_readiness_checker_verifies_public_evidence_artifacts() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    obom_bytes = _rootfs_obom_bytes()
    sbom_url = "https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json"
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    payloads = {
        sbom_url: sbom_bytes,
        obom_url: obom_bytes,
    }

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return module.FetchBytes(b"", f"unexpected fetch {url}")
        return module.FetchBytes(data)

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [sbom_url],
                },
            ],
        },
    }

    assert module.check_release_evidence("https://release.capsem.test", health) == []

    corrupted = json.loads(json.dumps(health))
    corrupted["evidence"]["vm_oboms"][0]["hash"] = "0" * 64
    failures = module.check_release_evidence("https://release.capsem.test", corrupted)
    assert (
        "VM OBOM evidence /assets/releases/2030.0101.1/arm64-obom.cdx.json blake3 mismatch"
        in failures
    )

    assert "def check_release_evidence" in script
    assert '"sha256", "host SBOM evidence", "spdx"' in script
    assert '"blake3", "VM OBOM evidence", "rootfs_cyclonedx"' in script
    assert "hashlib.sha256" in script
    assert "blake3.blake3" in script
    assert "attestation subject {subject} missing from published file lists" in script
    assert "attestation_predicate_evidence_urls" in script
    assert "attestation predicate_url {predicate_url} missing from {predicate_label}" in script
    assert "resolves published host SBOM and VM OBOM evidence artifacts" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates their SPDX 2.3 or CycloneDX document shape" in docs_text


def test_remote_release_readiness_rejects_live_host_obom_even_with_valid_cyclonedx() -> None:
    module = _readiness_checker_module()
    document = json.loads(_rootfs_obom_bytes())
    document["components"].append(
        {
            "type": "application",
            "name": "host browser extension",
            "properties": [{"name": "cdx:osquery:category", "value": "chrome_extensions"}],
        }
    )

    failure = module.validate_evidence_document(
        json.dumps(document).encode(),
        "rootfs_cyclonedx",
        "VM OBOM evidence",
        "/assets/releases/test/arm64-obom.cdx.json",
    )

    assert failure is not None
    assert "contains live-host inventory" in failure


def test_remote_release_readiness_rejects_unscoped_host_obom() -> None:
    module = _readiness_checker_module()
    document = json.loads(_rootfs_obom_bytes())
    document["metadata"]["component"] = {
        "type": "operating-system",
        "name": "Ubuntu",
        "version": "24.04",
    }

    failure = module.validate_evidence_document(
        json.dumps(document).encode(),
        "rootfs_cyclonedx",
        "VM OBOM evidence",
        "/assets/releases/test/arm64-obom.cdx.json",
    )

    assert failure is not None
    assert "must declare exported-rootfs scope" in failure


def test_remote_release_readiness_checker_verifies_vm_asset_file_content() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    workflow = _workflow_text("release-channel.yaml")
    rootfs_url = (
        "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs"
    )
    rootfs_bytes = b"rootfs-content"

    module.fetch_bytes = lambda url: module.FetchBytes(
        rootfs_bytes if url == rootfs_url else b"", None
    )
    item = {
        "arch": "arm64",
        "logical_name": "rootfs.erofs",
        "url": rootfs_url,
        "hash": module.blake3.blake3(rootfs_bytes).hexdigest(),
        "size": len(rootfs_bytes),
    }

    assert (
        module.fetch_and_verify_evidence_artifact(
            "https://release.capsem.org", item, "blake3", "VM asset file"
        )
        == []
    )

    item["hash"] = "0" * 64
    assert (
        f"VM asset file {rootfs_url} blake3 mismatch"
        in module.fetch_and_verify_evidence_artifact(
            "https://release.capsem.org", item, "blake3", "VM asset file"
        )
    )
    assert "fetch_and_verify_evidence_artifact(" in script
    assert '"VM asset file"' in script
    assert "uv run --project build_system --frozen python scripts/check-release-site-contract.py" in workflow


def test_remote_release_readiness_rejects_evidence_content_drift() -> None:
    module = _readiness_checker_module()
    bad_sbom_bytes = b'{"spdxVersion":"SPDX-2.2"}'
    bad_obom_bytes = b'{"bomFormat":"not-cyclonedx"}'
    package_url = "https://github.com/google/capsem/releases/download/v1.0.0/Capsem-1.0.0.pkg"
    sbom_url = "https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json"
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    payloads = {sbom_url: bad_sbom_bytes, obom_url: bad_obom_bytes}

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return module.FetchBytes(b"", f"unexpected fetch {url}")
        return module.FetchBytes(data)

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(bad_obom_bytes).hexdigest(),
                    "size": len(bad_obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(bad_obom_bytes).hexdigest(),
                    "size": len(bad_obom_bytes),
                }
            ],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(bad_sbom_bytes).hexdigest(),
                    "size": len(bad_sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "Capsem-1.0.0.pkg",
                    "url": package_url,
                    "sha256": "1" * 64,
                    "size": 42,
                },
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(bad_sbom_bytes).hexdigest(),
                    "size": len(bad_sbom_bytes),
                },
            ],
            "attestations": [
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [package_url],
                },
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)

    assert f"host SBOM evidence {sbom_url} spdxVersion mismatch" in failures
    assert f"VM OBOM evidence {obom_path} bomFormat mismatch" in failures


def test_release_rejects_sha1_only_spdx_file_checksums() -> None:
    module = _readiness_checker_module()
    sbom_url = "https://github.com/google/capsem/releases/download/v1.4.0/capsem-sbom.spdx.json"
    sha1_only_spdx = b"""{
      "spdxVersion": "SPDX-2.3",
      "files": [
        {
          "SPDXID": "SPDXRef-File-capsem-gateway",
          "checksums": [
            {
              "algorithm": "SHA1",
              "checksumValue": "2a2bebeee60f894f3599e06c755c91944f1c3cc8"
            }
          ]
        }
      ]
    }"""
    module.fetch_bytes = lambda url: module.FetchBytes(
        sha1_only_spdx if url == sbom_url else b"", None
    )
    item = {
        "name": "capsem-sbom.spdx.json",
        "url": sbom_url,
        "sha256": hashlib.sha256(sha1_only_spdx).hexdigest(),
        "size": len(sha1_only_spdx),
    }

    failures = module.fetch_and_verify_evidence_artifact(
        "https://release.capsem.org", item, "sha256", "host SBOM evidence", "spdx"
    )

    assert (
        f"host SBOM evidence {sbom_url} SPDX file SPDXRef-File-capsem-gateway "
        "missing SHA256 checksum"
    ) in failures
    script = _source_text("scripts/check-remote-release-readiness.py")
    assert "missing SHA256 checksum" in script
    assert 'algorithm.upper() == "SHA256"' in script


def test_remote_readiness_allows_first_channel_bootstrap_without_host_evidence() -> None:
    module = _readiness_checker_module()
    obom_bytes = _rootfs_obom_bytes()
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"

    def fake_fetch_bytes(url: str):
        if url == obom_url:
            return module.FetchBytes(obom_bytes)
        return module.FetchBytes(b"", f"unexpected fetch {url}")

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [],
            "host_binary_files": [],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                }
            ],
        },
    }

    assert module.check_release_evidence("https://release.capsem.test", health) == []

    with_binary_without_sbom = json.loads(json.dumps(health))
    with_binary_without_sbom["evidence"]["host_binary_files"] = [
        {
            "name": "Capsem-1.4.1.pkg",
            "url": "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg",
            "sha256": "0" * 64,
            "size": 123,
        }
    ]
    failures = module.check_release_evidence(
        "https://release.capsem.test", with_binary_without_sbom
    )
    assert "health evidence host_sboms missing for published binary files" in failures


def test_release_channel_smoke_and_remote_readiness_validate_matching_attestation_predicate_evidence() -> (
    None
):
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    workflow = _workflow_text("release-channel.yaml")
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    obom_bytes = _rootfs_obom_bytes()
    sbom_url = "https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json"
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    payloads = {
        sbom_url: sbom_bytes,
        obom_url: obom_bytes,
    }

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return module.FetchBytes(b"", f"unexpected fetch {url}")
        return module.FetchBytes(data)

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [sbom_url],
                },
            ],
        },
    }

    assert module.check_release_evidence("https://release.capsem.test", health) == []

    corrupted = json.loads(json.dumps(health))
    corrupted["evidence"]["attestations"][0]["predicate_url"] = (
        "/assets/releases/2030.0101.1/missing-obom.cdx.json"
    )
    assert (
        "attestation predicate_url /assets/releases/2030.0101.1/missing-obom.cdx.json "
        "missing from VM OBOM evidence"
    ) in module.check_release_evidence("https://release.capsem.test", corrupted)

    missing_predicate = json.loads(json.dumps(health))
    del missing_predicate["evidence"]["attestations"][0]["predicate_url"]
    assert "health evidence VM asset attestation predicate_url missing" in (
        module.check_release_evidence("https://release.capsem.test", missing_predicate)
    )

    assert "attestation_predicate_evidence_urls" in script
    assert '"VM OBOM evidence"' in script
    assert '"host SBOM evidence"' in script
    assert "VM asset attestation predicate_url missing" in script
    assert "missing from {predicate_label}" in script
    assert "uv run --project build_system --frozen python scripts/check-release-site-contract.py" in workflow


def test_remote_readiness_rejects_attestation_rail_drift() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    obom_bytes = _rootfs_obom_bytes()
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    module.fetch_bytes = lambda url: module.FetchBytes(
        obom_bytes if url == obom_url else b"",
        None if url == obom_url else f"unexpected fetch {url}",
    )
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [],
            "host_binary_files": [],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "host_binaries",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                }
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)

    assert "health evidence github_attestations_vm_assets scope mismatch" in failures
    assert "health evidence github_attestations_vm_assets workflow mismatch" in failures
    assert "attestation_expected_rails" in script
    assert "health evidence {attestation_name} scope mismatch" in script
    assert "health evidence {attestation_name} workflow mismatch" in script


def test_remote_readiness_rejects_host_sbom_attestation_subjects_missing_package() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    sbom_url = "https://github.com/google/capsem/releases/download/v1.4.1/capsem-sbom.spdx.json"
    pkg_url = "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg"

    def fake_fetch_bytes(url: str):
        if url == sbom_url:
            return module.FetchBytes(sbom_bytes)
        return module.FetchBytes(b"", f"unexpected fetch {url}")

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {"files": []},
        "evidence": {
            "vm_oboms": [],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "Capsem-1.4.1.pkg",
                    "url": pkg_url,
                    "sha256": "1" * 64,
                    "size": 123,
                },
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                },
            ],
            "attestations": [
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [sbom_url],
                }
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)
    assert (
        "health evidence host SBOM attestation subjects missing "
        "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg"
    ) in failures

    health["evidence"]["attestations"][0]["subjects"].append(pkg_url)
    assert module.check_release_evidence("https://release.capsem.test", health) == []

    assert "host_sbom_attestation_subjects" in script
    assert "github_attestations_host_sbom" in script
    assert "host SBOM attestation subjects missing" in script


def test_remote_readiness_rejects_noncanonical_host_sbom_evidence() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    sbom_url = "https://github.com/google/capsem/releases/download/v1.4.1/capsem-sbom.spdx.json"
    pkg_url = "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg"

    def fake_fetch_bytes(url: str):
        if url == sbom_url:
            return module.FetchBytes(sbom_bytes)
        return module.FetchBytes(b"", f"unexpected fetch {url}")

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {"files": []},
        "evidence": {
            "vm_oboms": [],
            "host_sboms": [
                {
                    "name": "not-the-canonical-sbom.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "Capsem-1.4.1.pkg",
                    "url": pkg_url,
                    "sha256": "1" * 64,
                    "size": 123,
                },
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                },
            ],
            "attestations": [
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [pkg_url],
                }
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)

    assert f"host SBOM evidence {sbom_url} name mismatch" in failures
    assert "health evidence host SBOM attestation predicate_url missing" in failures
    assert "host SBOM evidence {url} name mismatch" in script
    assert "host SBOM attestation predicate_url missing" in script


def test_release_channel_smoke_host_sbom_attestation_subjects_cover_packages() -> None:
    script = _source_text("scripts/check-remote-release-readiness.py")

    assert "host_sbom_attestation_subjects" in script
    assert "github_attestations_host_sbom" in script
    assert "host SBOM attestation subjects missing" in script


def test_remote_release_readiness_checker_verifies_live_cache_headers() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())
    calls: list[str] = []
    headers = {
        "https://release.capsem.test/": "no-cache, must-revalidate",
        "https://release.capsem.test/channels.json": "no-cache, must-revalidate",
        "https://release.capsem.test/assets/stable/manifest.json": "no-cache, must-revalidate",
        "https://release.capsem.test/assets/releases/2030.0101.1/arm64-rootfs.erofs": (
            "public, max-age=31536000, immutable"
        ),
    }

    def fake_fetch_headers(url: str):
        calls.append(url)
        cache_control = headers.get(url)
        if cache_control is None:
            return module.FetchHeaders({}, f"unexpected header fetch {url}")
        return module.FetchHeaders({"cache-control": cache_control})

    module.fetch_headers = fake_fetch_headers
    asset_files = [
        {
            "url": "/assets/releases/2030.0101.1/arm64-rootfs.erofs",
            "hash": "a" * 64,
            "size": 4,
        }
    ]
    assert (
        module.check_release_cache_headers("https://release.capsem.test", "stable", asset_files)
        == []
    )
    assert calls == list(headers)

    headers["https://release.capsem.test/assets/stable/manifest.json"] = (
        "public, max-age=31536000, immutable"
    )
    failures = module.check_release_cache_headers(
        "https://release.capsem.test", "stable", asset_files
    )
    assert (
        "channel manifest https://release.capsem.test/assets/stable/manifest.json "
        "Cache-Control must contain no-cache"
    ) in failures

    assert "def check_release_cache_headers" in script
    assert 'release_site_request(url, method="HEAD")' in script
    assert "RELEASE_VALIDATOR_USER_AGENT" in script
    assert "Cache-Control must contain {directive}" in script
    assert "max-age=31536000" in script
    assert "Cache-Control" in docs
    assert "mutable release-channel pointers" in docs_text
    assert (
        "immutable asset and profile artifacts" in docs_text
        or "immutable profile release artifacts" in docs_text
    )


def test_ci_installs_b3sum_before_bootstrap_asset_hash_checks() -> None:
    workflow = _workflow_job_block("test")

    import re
    import tomllib

    select_pos = workflow.find("build_system/scripts/ci/gate-tool-list.py")
    install_pos = workflow.find("- name: Install prebuilt Rust tools")
    bootstrap_pos = workflow.find("uv run --project build_system --frozen python -m pytest -c build_system/pyproject.toml tests/capsem-bootstrap/")

    assert select_pos != -1, "the job no longer derives its tools from config"
    assert install_pos != -1
    assert bootstrap_pos != -1
    # Derive, install, then use. The set is resolved before the installer runs
    # and both happen before anything hashes an asset.
    assert select_pos < install_pos < bootstrap_pos

    # And that the set it selects actually carries b3sum. Spelling the pin here
    # is what let the binary pairing gate go without it: this guard covered
    # every tool in one job, while its sibling covered one tool in every job.
    sets = tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))[
        "toolchain"
    ]["sets"]
    members: set[str] = set()
    for match in re.findall(r"--sets ([a-z,]+)", workflow):
        for label in match.split(","):
            members.update(sets[label])
    assert "b3sum" in members


def test_ci_provides_sha256sum_before_codecov_uploads_on_macos() -> None:
    workflow = _workflow_job_block("test")

    install_tools_pos = workflow.find("- name: Install sha256sum compatibility wrapper")
    sha256sum_pos = workflow.find("printf '%s\\n' '#!/bin/sh' 'exec shasum -a 256 \"$@\"'")
    codecov_pos = workflow.find("Upload Rust unit test coverage")

    assert install_tools_pos != -1
    assert sha256sum_pos != -1
    assert codecov_pos != -1
    assert install_tools_pos < sha256sum_pos < codecov_pos


def test_guest_network_doctor_is_hermetic_by_default() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "CAPSEM_RUN_PUBLIC_NETWORK_SMOKE" not in source
    assert "google.com" not in source
    assert "api.openai.com" not in source
    assert "api.anthropic.com" not in source
    assert "cdn.elie.net" not in source


def test_guest_network_doctor_exercises_oauth_fixture() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "/oauth/token" in source
    assert "grant_type=authorization_code" in source


def test_mock_server_helper_exports_https_fixture_for_host_callers() -> None:
    helper = (PROJECT_ROOT / "scripts" / "mock_server.py").read_text()

    assert "CAPSEM_MOCK_SERVER_HTTPS_BASE_URL" in helper
    assert "https_base_url" in helper
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in helper


def test_guest_network_doctor_requires_local_mock_server_instead_of_skipping() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()
    helper = source.split("def _require_local_mock_url", maxsplit=1)[1].split(
        "\n\n# ---------------------------------------------------------------",
        maxsplit=1,
    )[0]

    assert "pytest.skip" not in helper
    assert "pytest.fail" in helper
    assert "LOCAL_MOCK_SERVER_ENV" in helper
    assert 'LOCAL_MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"' in source


def test_guest_network_doctor_has_no_skipped_protocol_proofs() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "pytest.skip" not in source


def test_doctor_session_validation_starts_mock_server() -> None:
    source = (
        PROJECT_ROOT
        / "build_system/builder/gate/tools/doctor/doctor_session_test.py"
    ).read_text()

    assert "_mock_server_module" in source
    assert 'SCRIPT_DIR / "mock_server.py"' in source
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in source
    assert '"create",' in source
    assert '"exec",' in source
    assert '"-e",' in source
    assert 'f"{MOCK_SERVER_ENV}={mock_base_url}"' in source
    assert "PERSISTENT_DIR" in source
    assert '"capsem-doctor"' in source


def test_release_scripts_use_shared_mock_server_helper() -> None:
    helper = PROJECT_ROOT / "scripts" / "mock_server.py"
    assert helper.exists(), "release scripts need one shared mock-server helper"

    doctor = (
        PROJECT_ROOT
        / "build_system/builder/gate/tools/doctor/doctor_session_test.py"
    ).read_text()
    assert "_mock_server_module" in doctor
    assert 'SCRIPT_DIR / "mock_server.py"' in doctor
    assert "def _read_mock_server_ready" not in doctor
    assert "def _start_mock_server" not in doctor

    direct_imports = ["scripts/integration_test.py"]
    helper_imports = [
        "tests/capsem-serial/test_mock_server_protocol_benchmark.py",
    ]
    for rel in direct_imports:
        source = (PROJECT_ROOT / rel).read_text()
        assert "from mock_server import" in source
        assert "def _read_mock_server_ready" not in source
        assert "def _start_mock_server" not in source
    for rel in helper_imports:
        source = (PROJECT_ROOT / rel).read_text()
        assert "from helpers.mock_server import" in source
        assert "def _read_mock_server_ready" not in source
        assert "def _start_mock_server" not in source


def test_mock_server_is_the_only_hermetic_fixture_server_contract() -> None:
    current_files = [
        PROJECT_ROOT / "scripts" / "mock_server.py",
        PROJECT_ROOT / "tests" / "helpers" / "mock_server.py",
        PROJECT_ROOT / "crates" / "capsem-mock-server" / "src" / "main.rs",
        PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "__main__.py",
        PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "helpers.py",
    ]

    for path in current_files:
        text = path.read_text()
        assert OLD_DEBUG_CRATE not in text
        assert "debug_upstream" not in text
        assert "CAPSEM_BENCH_MOCK_SERVER_PROTOCOL_BASE_URL" not in text

    assert (PROJECT_ROOT / "crates" / OLD_DEBUG_CRATE).exists() is False
    assert (PROJECT_ROOT / "crates" / "capsem-mock-server").exists()
    assert not list((PROJECT_ROOT / "scripts").glob("*mock_server_impl*"))
    assert (PROJECT_ROOT / "scripts" / "debug_upstream.py").exists() is False
    assert (PROJECT_ROOT / "tests" / "helpers" / "debug_upstream.py").exists() is False


def test_ci_workflow_references_only_live_workspace_packages_and_skills() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=PROJECT_ROOT,
            text=True,
        )
    )
    packages = {package["name"] for package in metadata["packages"]}
    referenced = set(re.findall(r"(?:^|\\s)-p\\s+([a-z0-9_-]+)", workflow))
    unknown = sorted(referenced - packages)

    assert unknown == []
    assert OLD_DEBUG_CRATE not in workflow
    assert "validate-skills skills" in workflow
    assert "validate-skills config/skills" not in workflow


def test_ci_builds_frontend_before_compiling_tauri_app_tests() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    build_pos = workflow.find("bash scripts/check-web-surface.sh frontend-build")
    capsem_app_pos = workflow.find("-p capsem-app")
    coverage_pos = workflow.rfind("cargo llvm-cov nextest --no-cfg-coverage", 0, capsem_app_pos)

    assert build_pos != -1, "Tauri frontendDist must exist before capsem-app tests compile"
    assert coverage_pos != -1
    assert capsem_app_pos != -1
    assert build_pos < coverage_pos < capsem_app_pos


def test_frontend_generated_settings_use_one_shared_rail() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    binary_release = _workflow_text("release.yaml")
    profile_release = _workflow_text("release-assets.yaml")
    fast_gate = _workflow_text("fast-gate.yaml")
    just = (PROJECT_ROOT / "justfile").read_text()
    web_gate = _source_text("scripts/check-web-surface.sh")

    generate_pos = workflow.find("bash scripts/generate-settings.sh")
    first_frontend_build_pos = workflow.find("bash scripts/check-web-surface.sh frontend-build")
    frontend_check_pos = workflow.find("bash scripts/check-web-surface.sh frontend")

    assert generate_pos != -1
    assert first_frontend_build_pos != -1
    assert frontend_check_pos != -1
    assert "uses: ./.github/workflows/fast-gate.yaml" in binary_release
    assert "uses: ./.github/workflows/fast-gate.yaml" in profile_release
    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate
    gate = _dispatched_text("test-clean:")
    assert "bootstrap.sh" in gate
    assert "check-web-surface.sh frontend" in gate
    assert "pnpm --dir web/app run check" in web_gate
    assert generate_pos < first_frontend_build_pos
    assert generate_pos < frontend_check_pos
    assert "bash scripts/generate-settings.sh" in just
    generated_gate = _recipe_block("_check-generated-settings:")
    assert "check-generated-settings.sh" in generated_gate
    assert "_dev-frontend: _pnpm-install _generate-settings" in just
    assert '_build-ui profile="debug": _pnpm-install _generate-settings' in just
    assert "\ntest-frontend:" not in just
    assert "uv run --project build_system --frozen python scripts/generate_schema.py" not in just


def test_generated_settings_gate_bootstraps_ignored_output_and_rejects_tracked_drift(
    tmp_path: Path,
) -> None:
    """A clean checkout has no ignored frontend mock, but drift still fails closed."""
    root = tmp_path / "clean-checkout"
    scripts = root / "scripts"
    settings = root / "config/settings"
    frontend = root / "web/app/src/lib"
    scripts.mkdir(parents=True)
    settings.mkdir(parents=True)
    frontend.mkdir(parents=True)

    shutil.copy2(
        PROJECT_ROOT / "scripts/check-generated-settings.sh",
        scripts / "check-generated-settings.sh",
    )
    (scripts / "generate-settings.sh").write_text(
        """#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# `$1` is where the tracked pair goes. The checker passes a scratch directory
# so the gate never rewrites its own checked-in source; the mock is gitignored
# and still lands in the checkout, because the web checks import it.
OUT="${1:-$ROOT/config/settings}"
mkdir -p "$ROOT/web/app/src/lib" "$OUT"
if [ "${FAKE_SKIP_RUNTIME_OUTPUT:-0}" != 1 ]; then
  printf 'runtime mock\n' > "$ROOT/web/app/src/lib/mock-settings.generated.ts"
fi
if [ "${FAKE_TRACKED_DRIFT:-0}" = 1 ]; then
  printf 'drifted schema\n' > "$OUT/schema.generated.json"
else
  cp "$ROOT/config/settings/schema.generated.json" "$OUT/schema.generated.json"
fi
cp "$ROOT/config/settings/ui-metadata.generated.json" "$OUT/ui-metadata.generated.json"
"""
    )
    schema = settings / "schema.generated.json"
    metadata = settings / "ui-metadata.generated.json"
    runtime_mock = frontend / "mock-settings.generated.ts"
    schema.write_text("checked-in schema\n")
    metadata.write_text("checked-in metadata\n")
    assert not runtime_mock.exists(), "fixture must model a clean git checkout"

    clean = subprocess.run(
        ["bash", str(scripts / "check-generated-settings.sh"), str(root)],
        text=True,
        capture_output=True,
        check=False,
    )
    assert clean.returncode == 0, clean.stderr
    assert runtime_mock.read_text() == "runtime mock\n"
    assert schema.read_text() == "checked-in schema\n"
    assert metadata.read_text() == "checked-in metadata\n"

    runtime_mock.unlink()
    missing_env = os.environ.copy()
    missing_env["FAKE_SKIP_RUNTIME_OUTPUT"] = "1"
    missing = subprocess.run(
        ["bash", str(scripts / "check-generated-settings.sh"), str(root)],
        env=missing_env,
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing.returncode != 0
    assert (
        "settings generator did not create: web/app/src/lib/mock-settings.generated.ts"
    ) in missing.stderr

    drift_env = os.environ.copy()
    drift_env["FAKE_TRACKED_DRIFT"] = "1"
    drifted = subprocess.run(
        ["bash", str(scripts / "check-generated-settings.sh"), str(root)],
        env=drift_env,
        text=True,
        capture_output=True,
        check=False,
    )
    assert drifted.returncode != 0
    assert "generated settings drifted: config/settings/schema.generated.json" in (drifted.stderr)


def test_settings_generator_uses_current_config_authority() -> None:
    generator = (PROJECT_ROOT / "scripts" / "generate_schema.py").read_text()

    assert 'PROJECT_ROOT / "config" / "docker" / "image"' in generator
    assert 'PROJECT_ROOT / "guest"' not in generator
    assert '"guest/config"' not in generator


def test_runtime_credential_store_does_not_use_native_keychain() -> None:
    runtime_files = [
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "credential_broker.rs",
        PROJECT_ROOT / "crates" / "capsem" / "src" / "service_install.rs",
        PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs",
        PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs",
        PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "main.rs",
    ]
    forbidden = [
        "CAPSEM_CREDENTIAL_BROKER_TEST_STORE",
        "org.capsem.credentials",
        "com.capsem.credential",
        "credential_store_backend_native",
        "durable_store_write_native",
        "durable_store_read_native",
        "durable_store_hydrate_native",
        "security find-generic-password",
        "security add-generic-password",
        "security delete-generic-password",
        "keyring::",
        "security_framework",
        "SecKeychain",
    ]

    for path in runtime_files:
        source = path.read_text()
        for needle in forbidden:
            assert needle not in source, f"{path} must not call native Keychain storage"

    broker = runtime_files[0].read_text()
    assert "CAPSEM_CREDENTIAL_STORE_PATH" in broker
    assert "default_credential_store_path()" in broker


def test_installer_codesigns_helpers_with_stable_identifiers() -> None:
    """Dev/package helper signatures must not get hash-derived identities.

    Hash-derived ad-hoc identifiers make macOS authorization prompts repeat
    after every rebuild. The installed helper binaries use stable Capsem
    identifiers even when the signing identity is ad-hoc in local/dev builds.
    """

    postinstall = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "pkg-scripts" / "postinstall").read_text()
    simulate_install = (PROJECT_ROOT / "scripts" / "simulate-install.sh").read_text()
    expected = [
        "org.capsem.cli",
        "org.capsem.service",
        "org.capsem.process",
        "org.capsem.tui",
        "org.capsem.mcp",
        "org.capsem.mcp.aggregator",
        "org.capsem.mcp.builtin",
        "org.capsem.gateway",
        "org.capsem.tray",
        "org.capsem.admin",
        "org.capsem.mock-server",
    ]

    for script in [postinstall, simulate_install]:
        assert "codesign_identifier_for_bin()" in script
        assert 'codesign --sign - --identifier "$identifier"' in script
        for identifier in expected:
            assert identifier in script


def test_binary_update_installer_scripts_replace_and_restart_full_helper_cohort() -> None:
    preinstall = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "pkg-scripts" / "preinstall").read_text()
    retire_cohort = (
        PROJECT_ROOT / "build_system" / "packaging" / "shared" / "retire-cohort"
    ).read_text()
    postinstall = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "pkg-scripts" / "postinstall").read_text()
    linux = PROJECT_ROOT / "build_system" / "packaging" / "linux"
    deb_preinst = (linux / "deb-preinst.sh").read_text()
    deb_postinst = (linux / "deb-postinst.sh").read_text()
    repack_deb = (linux / "repack-deb.sh").read_text()
    required_bins = [
        "capsem",
        "capsem-service",
        "capsem-process",
        "capsem-tui",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-gateway",
        "capsem-tray",
        "capsem-admin",
        "capsem-mock-server",
    ]
    stale_companions = [
        "capsem-service",
        "capsem-gateway",
        "capsem-tray",
        "capsem-process",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
    ]

    assert 'launchctl bootout "gui/$(id -u "$USER")" "$PLIST"' in preinstall
    assert "launchctl unload" in preinstall
    for name in stale_companions:
        assert name in retire_cohort
    for caller in (preinstall, deb_preinst):
        assert "retire-cohort" in caller
        assert "capsem_retire_native_cohort" in caller
    assert "capsem_process_is_package_owned" in retire_cohort
    assert '"$kill_command" -9 "$pid"' in retire_cohort
    assert "retired native helper" in retire_cohort
    assert "pkill -9 -x capsem-app" in preinstall
    assert "systemctl --user stop capsem.service" in deb_preinst
    assert "event=stop_systemd_user_service" in deb_preinst
    assert 'cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"' in repack_deb
    assert 'chmod 755 "$WORK_DIR/deb/DEBIAN/preinst"' in repack_deb
    assert "embed_native_cohort_retirement" in repack_deb
    assert 'rm -rf "$USER_HOME/Applications/Capsem.app"' in preinstall
    assert "rm -rf /Applications/Capsem.app" in preinstall
    assert "rm -rf /usr/local/share/capsem" in preinstall

    for script in (postinstall, deb_postinst):
        for name in required_bins:
            assert name in script
        assert "update --assets" in script
        assert "event=manifest_installed" in script

    assert 'src="$PKG_SHARE/bin/$bin"' in postinstall
    assert 'cp "$src" "$CAPSEM_DIR/bin/$bin"' in postinstall
    assert 'su "$USER" -c "$CAPSEM_DIR/bin/capsem install"' in postinstall
    assert "event=service_registered" in postinstall
    assert 'grep -q "Service:   ok"' in postinstall
    assert 'grep -q "Gateway:   ok"' in postinstall
    assert 'su "$USER" -c "open /Applications/Capsem.app"' in postinstall
    assert "capsem-tray &" not in postinstall
    assert "event=service_not_ready" in postinstall

    assert 'ln -sf "/usr/bin/$bin" "$CAPSEM_DIR/bin/$bin"' in deb_postinst
    assert 'su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR $CAPSEM_DIR/bin/capsem install"' in (
        deb_postinst
    )
    assert "event=service_install_invoked" in deb_postinst
    assert 'capsem install" 2>/dev/null || true' not in deb_postinst
    assert "event=service_registration_failed" in deb_postinst
    assert "event=readiness_poll" in deb_postinst
    assert 'grep -q "Service:   ok"' in deb_postinst
    assert 'grep -q "Gateway:   ok"' in deb_postinst
    assert "event=service_not_ready" in deb_postinst


def test_helper_version_surfaces_support_installed_update_smoke() -> None:
    """Helper binaries must expose --version so update smokes can prove cohort drift."""

    for path, struct_name in [
        ("crates/capsem-admin/src/main.rs", "Cli"),
        ("crates/capsem-mcp-aggregator/src/main.rs", "Args"),
        ("crates/capsem-gateway/src/main.rs", "Args"),
        ("crates/capsem-tray/src/main.rs", "Args"),
    ]:
        command = _command_attribute_prefix(_source_text(path), struct_name)
        assert "#[command" in command and "version" in command, path

    for path, binary in [
        ("crates/capsem-mcp/src/main.rs", "capsem-mcp"),
        ("crates/capsem-mcp-builtin/src/main.rs", "capsem-mcp-builtin"),
    ]:
        source = _source_text(path)
        assert 'arg == "--version" || arg == "-V"' in source, path
        assert f'println!("{binary} {{}}", env!("CARGO_PKG_VERSION"))' in source, path


def test_desktop_shell_does_not_run_native_updater_or_background_https_check() -> None:
    """The GUI must not perform hidden native updater HTTPS work on startup.

    In 1.3 update checks go through the explicit service `/update/check` route.
    The Tauri updater plugin brings its own HTTP stack and platform verifier,
    which can touch macOS Keychain/trust APIs outside Capsem's service logs.
    """

    app_manifest = (PROJECT_ROOT / "crates" / "capsem-app" / "Cargo.toml").read_text()
    app_source = (PROJECT_ROOT / "crates" / "capsem-app" / "src" / "main.rs").read_text()
    tauri_conf = (PROJECT_ROOT / "crates" / "capsem-app" / "tauri.conf.json").read_text()
    capabilities = (
        PROJECT_ROOT / "crates" / "capsem-app" / "capabilities" / "default.json"
    ).read_text()

    forbidden = [
        "tauri-plugin-updater",
        "tauri_plugin_updater",
        "UpdaterExt",
        "check_for_update_with_prompt",
        "check_for_app_update",
        "createUpdaterArtifacts",
        '"updater"',
        "updater:default",
    ]
    for text in [app_manifest, app_source, tauri_conf, capabilities]:
        for needle in forbidden:
            assert needle not in text


def test_rust_http_stack_uses_webpki_roots_not_platform_keychain_verifier() -> None:
    """Runtime HTTP clients must not pull macOS platform trust/keychain APIs."""

    manifest = (PROJECT_ROOT / "Cargo.toml").read_text()
    reqwest_line = next(line for line in manifest.splitlines() if line.startswith("reqwest = "))
    assert 'version = "0.12"' in reqwest_line
    assert "rustls-tls-webpki-roots" in reqwest_line
    assert '"rustls"' not in reqwest_line

    service_manifest = (PROJECT_ROOT / "crates" / "capsem-service" / "Cargo.toml").read_text()
    ort_line = next(line for line in service_manifest.splitlines() if line.startswith("ort = "))
    assert "default-features = false" in ort_line
    assert '"tls-rustls"' in ort_line
    assert '"tls-native"' not in ort_line

    for package in ["rustls-platform-verifier", "native-tls", "security-framework"]:
        result = subprocess.run(
            ["cargo", "tree", "-i", package, "--workspace", "--edges", "all"],
            cwd=PROJECT_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        if result.returncode != 0:
            assert "did not match any packages" in result.stdout
            continue
        assert package not in result.stdout


def test_stop_command_stays_before_status_and_credential_hydration() -> None:
    source = (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()

    stop_arm = re.search(
        r"Commands::Misc\(MiscCommands::Stop\) => \{\n(?P<body>.*?)\n        \}",
        source,
        re.DOTALL,
    )
    assert stop_arm is not None
    body = stop_arm.group("body")

    assert "service_install::stop_service().await?" in body
    assert 'println!("Service stopped.");' in body
    assert "return Ok(());" in body

    forbidden = [
        "UdsClient",
        "client::UdsClient",
        "service_json",
        "/profiles/status",
        "/corp/info",
        "/vms/list",
        "credential",
        "status_client",
        "list_client",
        "try_ensure_service",
    ]
    for needle in forbidden:
        assert needle not in body, f"`capsem stop` must not touch {needle}"

    client_creation = re.search(
        r"^\s*let client = (?:client::)?UdsClient::[a-z_][a-z0-9_]*\(",
        source,
        re.MULTILINE,
    )
    stop_position = source.find("Commands::Misc(MiscCommands::Stop)")
    assert stop_position != -1
    assert client_creation is not None
    assert stop_position < client_creation.start()


def test_changelog_does_not_advertise_keychain_credential_storage_for_1_3() -> None:
    changelog = (PROJECT_ROOT / "CHANGELOG.md").read_text()
    section = changelog.split("## [1.3.1782571508]", maxsplit=1)[1].split("\n## [", maxsplit=1)[0]

    assert "Disabled the macOS Keychain-backed credential broker store" in section
    assert "file-backed durable storage" in section
    assert "Added credential broker plugin support with Keychain-backed storage" not in section
    assert "single `org.capsem.credentials` Keychain vault item" not in section
    assert "credential store/keychain" not in section


def test_release_docs_identify_body_blobs_as_forensic_truth() -> None:
    telemetry = (
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "session-telemetry.md"
    ).read_text()
    network = (
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "security" / "network-isolation.md"
    ).read_text()
    debug_skill = (PROJECT_ROOT / "skills" / "dev-session-debug" / "SKILL.md").read_text()
    mcp_skill = (PROJECT_ROOT / "skills" / "dev-mcp" / "SKILL.md").read_text()

    for text in (telemetry, network, debug_skill, mcp_skill):
        assert "event_body_blobs" in text

    assert "forensic" in telemetry
    assert "body truth is in `event_body_blobs`" in telemetry
    assert "blob table is the ledger" in telemetry
    assert "blob table is the forensic body source" in network
    assert "not the forensic source of truth" in debug_skill
    assert "MCP-only body rail" in mcp_skill

    stale_claims = [
        "| `request_body_preview` | TEXT | First 4 KB of request body |",
        "| `response_body_preview` | TEXT | First 4 KB of response body |",
        "| `request_body_preview` | First 4 KB of request body |",
        "| `response_body_preview` | First 4 KB of response body |",
        "request_preview TEXT,              -- first 256KB",
        "response_preview TEXT,             -- first 256KB",
    ]
    combined = "\n".join([telemetry, network, debug_skill, mcp_skill])
    for claim in stale_claims:
        assert claim not in combined


def test_release_docs_reject_old_service_routes_and_manifest_signing() -> None:
    architecture_skill = _skill_text("skills/site-architecture/SKILL.md")
    release_skill = _skill_text("skills/release-process/SKILL.md")

    current_service_table = architecture_skill.split("### Service HTTP API", maxsplit=1)[1].split(
        "### MCP tools", maxsplit=1
    )[0]
    for retired in [
        "`/provision`",
        "`/list`",
        "`/info/{id}`",
        "`/stop/{id}`",
        "`/resume/{name}`",
        "`/persist/{id}`",
        "`/write_file/{id}`",
        "`/read_file/{id}?path=...`",
    ]:
        assert retired not in current_service_table

    assert "`/vms/create`" in current_service_table
    assert "`/vms/list`" in current_service_table
    assert "`/vms/{id}/status`" in current_service_table
    assert "Unknown routes must\nreturn 404" in current_service_table

    assert "Do not resurrect local VM manifest signing" in release_skill
    for stale in [
        "Install manifest-signing tools before signing",
        "Local manifest signing is part of setup",
        "bootstrap.sh` must install `minisign`",
        "Sign package payload manifest",
    ]:
        assert stale not in release_skill


def test_release_docs_name_tool_calls_as_canonical_tool_ledger() -> None:
    docs = [
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "mcp-gateway.md",
        PROJECT_ROOT
        / "docs"
        / "src"
        / "content"
        / "docs"
        / "architecture"
        / "session-telemetry.md",
        PROJECT_ROOT / "skills" / "dev-mcp" / "SKILL.md",
        PROJECT_ROOT / "skills" / "dev-session-debug" / "SKILL.md",
    ]
    combined = "\n".join(path.read_text() for path in docs)

    assert "tool_calls` is the canonical" in combined
    assert "mcp_calls" not in combined
    assert "An MCP `tools/call` without a matching" in combined


def test_frontend_coverage_runner_declares_its_provider() -> None:
    package_json = json.loads((PROJECT_ROOT / "web" / "app" / "package.json").read_text())

    assert "@vitest/coverage-v8" in package_json["devDependencies"]


def test_frontend_coverage_artifacts_are_not_typechecked_or_misuploaded() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    tsconfig = json.loads((PROJECT_ROOT / "web" / "app" / "tsconfig.json").read_text())

    assert "target/coverage/web-app/coverage-final.json" in workflow
    assert "web/app/coverage/coverage-final.json" not in workflow
    assert "coverage" in tsconfig["exclude"]


def test_pr_ci_coverage_reports_without_local_threshold_abort() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    unit_step = next(
        step
        for step in _workflow_job("test")["steps"]
        if step.get("name") == "Unit tests with coverage"
    )
    report_command = next(
        line.strip()
        for line in unit_step["run"].splitlines()
        if line.strip().startswith("cargo llvm-cov report")
    )

    assert "--fail-under-lines" not in workflow
    assert "cargo llvm-cov report --no-cfg-coverage" not in workflow
    for test_selector in (
        "--lib",
        "--bins",
        "--tests",
        "--test",
        "--benches",
        "--examples",
        "--all-targets",
        "--doc",
    ):
        assert test_selector not in report_command.split()
    assert "target/coverage/rust/unit.json" in workflow
    assert "target/coverage/rust/summary.txt" in workflow
    assert "target/coverage/linux/codecov.json" in workflow
    assert "target/coverage/linux/summary.txt" in workflow


def test_linux_ci_coverage_cannot_hang_without_a_named_failure() -> None:
    workflow = _workflow_job_block("test-linux")
    runner = _source_text("scripts/test-linux-rust.sh")
    nextest = tomllib.loads((PROJECT_ROOT / ".config" / "nextest.toml").read_text())

    coverage_step = workflow.split("- name: Unit tests (KVM backend) with coverage", maxsplit=1)[
        1
    ].split("- name: Upload Linux coverage", maxsplit=1)[0]
    slow_timeout = nextest["profile"]["ci"]["slow-timeout"]

    assert "timeout-minutes:" in coverage_step
    assert "run: just test-linux-rust" in coverage_step
    assert "cargo llvm-cov nextest" in runner
    report_block = runner.split("cargo llvm-cov report", maxsplit=1)[1]
    assert "--bins" not in report_block
    assert "--fail-under-lines" not in runner
    from capsem_builder.gate import config as gate_config

    floor = gate_config.load(PROJECT_ROOT).modules.rust_coverage_floor
    assert floor.replace("=", " ") == RUST_LINE_COVERAGE_FLOOR
    assert "--profile ci" in runner
    assert slow_timeout == {
        "period": "120s",
        "terminate-after": 3,
        "grace-period": "10s",
        "on-timeout": "fail",
    }


def test_just_test_owns_linux_rust_platform_coverage_through_docker(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.hostimage import cargo_tool
    from helpers.gate import gate_issued, gate_plan

    canonical_gate = _dispatched_text("test-clean:")
    linux_rust_recipe = _recipe_body("_gate-linux-rust:")
    linux_ci = _workflow_job_block("test-linux")
    runner = _source_text("scripts/test-linux-rust.sh")
    host_builder = _source_text("build_system/docker/Dockerfile.host-builder")

    # Linux owns its cfg(target_os = "linux") coverage natively. macOS owns
    # the same checked-in runner through the sealed Docker lane, including the
    # builder/base dependency chain. Exercise both real plans instead of
    # asking the current host's Just recipe to contain the other platform.
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem_builder.gate.host.machine", lambda: "x86_64")
    native = gate_plan("linux-rust")
    assert native.labels == ("linux-rust",)
    assert "test-linux-rust.sh" in native.step_named("linux-rust").render()[0]

    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem_builder.gate.host.machine", lambda: "arm64")
    sealed = gate_plan("linux-rust")
    assert {"host-image", "warm-base", "linux-rust"} <= set(sealed.labels)
    assert sealed.after_of("warm-base") == {"host-image"}
    assert sealed.after_of("linux-rust") == {"warm-base"}
    linux_rust_gate = gate_issued("linux-rust")

    assert "capsem-host-builder" in canonical_gate
    assert "test-linux-rust.sh" in canonical_gate
    assert "capsem-gate linux-rust" in linux_rust_recipe
    assert "capsem-host-builder:latest" in linux_rust_gate

    # Reimplemented, not deleted. Six assertions here pinned the mechanism --
    # `docker run --rm`, `--user`, `/src:ro`, and two named volumes -- and the
    # lane no longer has any of them: it copies its source into an image,
    # resolves dependencies from a base image keyed by the lockfiles, and runs
    # sealed. The property each one protected still holds, expressed against
    # what the lane does now, and two of them are strictly stronger.
    #
    # `--rm` could not coexist with `docker cp`, so the container is created,
    # started, copied from, and removed -- removal on the failure path too,
    # which `--rm` could never give while still yielding the coverage of a lane
    # that failed.
    assert "docker create" in linux_rust_gate
    assert "docker start" in linux_rust_gate
    assert "docker cp" in linux_rust_gate
    assert linux_rust_gate.index("docker cp") < linux_rust_gate.rindex("docker rm"), (
        "the container is removed before its coverage is copied out"
    )

    # `/src:ro` said the container could not write the checkout. Nothing is
    # mounted at all now, which is the stronger claim and the one that ends the
    # race with the host steps that share those inodes.
    assert "/src:ro" not in linux_rust_gate
    # Only the commands that can mount. `docker rm -f -v` also carries a `-v`
    # and it means the opposite -- take the anonymous volumes with the
    # container -- so reading it as a mount fails this for the teardown doing
    # its job. The sibling guard in `test_gate_linuxrust_hermetic` says the
    # same thing about the same argv.
    mounting = [
        line
        for line in linux_rust_gate.splitlines()
        if line.strip().startswith(("docker create", "docker run"))
        and (" -v " in line or " --volume " in line)
    ]
    assert not mounting, "the parity lane grew a mount:\n  " + "\n  ".join(mounting)

    # The named volumes carried the cargo registry, the rustup toolchain and an
    # 11 GB target between runs -- the cross-run state that let a warm machine
    # and a clean checkout disagree about one commit. They live in a base image
    # keyed by the lockfiles that determine them.
    for volume in ("capsem-linux-rust-cargo-registry", "capsem-linux-rust-rustup"):
        assert volume not in linux_rust_gate, f"{volume} came back"
    assert "capsem-linux-rust-base:" in linux_rust_gate

    # `--user` kept the container off root, because the suite chmods an asset
    # to 0o000 and demands the read fail. That is now baked into the image.
    lane_dockerfile = _source_text("build_system/docker/Dockerfile.linux-rust")
    assert "USER 1000:1000" in lane_dockerfile

    # And the property none of the originals asserted, because it was not true:
    # the lane runs with no outbound network, which is what proved the mid-run
    # `pnpm install` and the `cdn.pyke.io` fetch inside `ort`'s build script
    # were there at all.
    assert "--network none" in linux_rust_gate

    # `nextest` moved out of the argv with the mount that bound its state; the
    # script the container runs is still the checked-in one, asserted below.
    assert "test-linux-rust.sh" in linux_rust_gate
    # Both hosts use the same sealed Docker lane. That makes local Linux and
    # Colima exercise one input-keyed dependency image instead of two subtly
    # different native/container implementations.
    linuxrust = _source_text("build_system/builder/gate/linuxrust.py")
    assert "Docker(context.runner)" in linuxrust
    assert "host.on_linux()" not in linuxrust
    assert "host.on_macos()" not in linuxrust
    assert "run: just test-linux-rust" in linux_ci
    assert "cargo llvm-cov nextest" not in linux_ci
    assert "cargo llvm-cov nextest" in runner
    linux_clippy = "cargo clippy --workspace --all-targets -- -D warnings"
    cross_clippy = (
        'cargo clippy --target "$cross_target" -p capsem-core --lib --tests -- -D warnings'
    )
    assert (
        "cross_target=$(python3 scripts/provision-linux-workspace.py --cross-rust-target)" in runner
    )
    assert cross_clippy in runner
    assert linux_clippy in runner
    assert runner.index(cross_clippy) < runner.index(linux_clippy)
    assert runner.index(linux_clippy) < runner.index("cargo llvm-cov nextest")
    assert "capsem-service" in runner
    assert 'package_args+=( -p "$package" )' in runner
    assert "--profile ci" in runner
    config = gate_config.load(PROJECT_ROOT)
    for argument, tool_name in config.hostimage.cargo_tool_args.items():
        package, version = cargo_tool(config=config, argument=argument)
        configured = next(tool for tool in config.toolchain.crates if tool.name == tool_name)
        assert package == configured.install[2]
        assert version == configured.install[configured.install.index("--version") + 1]
        assert f"ARG {argument}" in host_builder
        assert f'cargo install {package} --version "${{{argument}}}" --locked' in host_builder
        assert version not in host_builder, (
            f"{package}'s version must remain config-owned, not duplicated in Docker"
        )
    assert host_builder.count("for attempt in 1 2 3") >= len(config.hostimage.cargo_tool_args)
    assert "CARGO_NET_RETRY=10" in host_builder


def test_just_test_builds_real_host_packages_and_runs_production_sbom() -> None:
    canonical_gate = _dispatched_text("test-clean:")
    mac_glowup = _source_text("build_system/packaging/macos/macos_release_glowup.py")
    host_sbom = _recipe_block("_gate-host-package-sbom:")
    release = _source_text(".github/workflows/release.yaml")

    assert "package.arm64" in canonical_gate
    assert "package.x86_64" in canonical_gate
    assert "test-macos-install:" not in _source_text("justfile")
    # The step, not the script's argv. `_recipe_block` *runs* the plan against
    # a recording runner to capture real arguments, and `glowup.host-sbom` is a
    # `Call` that renders as prose -- so the script name only ever appeared
    # because a warm tree let the recorded run get that deep. On a checkout
    # with no built assets the run stops at the asset lanes and the name is
    # simply absent, which made this assert a fact about the machine rather
    # than about the gate.
    #
    # Named step plus the config it is built from is the same claim without
    # that dependency, and it is checked against the production script below
    # in `host_sbom` as well.
    assert "glowup.host-sbom" in canonical_gate
    assert "tests/capsem-recipes/" in canonical_gate
    assert "config.install.local_macos_package_script" in mac_glowup
    assert 'str(Path(__file__).resolve().parent / "macos_tart_glowup.py")' in mac_glowup
    assert 'str(Path(__file__).resolve().parent / "prove-macos-package-boot.sh")' in mac_glowup
    # Same reason as `glowup.host-sbom` above: the recipe dispatches to the
    # gate, and the command's work is a `Call` that describes itself in prose.
    # The script name reached this string only when a warm tree let the
    # recorded run reach the argv. The dispatch is the part this block can
    # honestly assert; `sbom.script` below is what it dispatches into.
    assert "capsem-gate host-sbom" in host_sbom
    assert "SBOM" in host_sbom
    # Exactly the current version's packages, so an older `.deb` still in
    # `target/packages/` cannot be described by a cohort nobody ships.
    from capsem_builder.gate import config as gate_config

    sbom = gate_config.load(PROJECT_ROOT).sbom
    assert "{version}" in sbom.linux_packages_glob
    assert sbom.expected_debs == 2
    # What `glowup.host-sbom` above is built from, so naming the step is still
    # a claim about running the production generator rather than about a step
    # label that could be wired to anything.
    assert sbom.script == "scripts/generate-host-binary-sbom.py"
    assert "build_system/packaging/macos/build-pkg.sh" in release
    assert "scripts/generate-host-binary-sbom.py" in release


def test_release_packages_use_exact_manifest_selected_profile_inputs() -> None:
    release = _source_text(".github/workflows/release.yaml")
    mac_job = _workflow_job_block("build-app-macos", "release.yaml")
    linux_job = _workflow_job_block("build-app-linux", "release.yaml")
    materializer = _source_text("scripts/materialize-config.sh")

    assert 'manifest_schema="release"' in materializer
    assert 'profile_path="$CONFIG_ROOT/profiles/$profile_id/profile.toml"' in materializer
    assert 'profile_paths=("$CONFIG_ROOT"/profiles/*/profile.toml)' in materializer
    assert "name: binary-channel-source" in mac_job
    assert "Fetch exact selected arm64 profiles" in mac_job
    assert "uses: ./.github/actions/fetch-release-inputs" in mac_job
    assert "architecture: arm64" in mac_job
    assert "output: target/binary-selected-profiles" in mac_job
    assert "--input-dir target/binary-selected-profiles" in mac_job
    assert "--assets-dir target/release-assets" in mac_job
    assert "--config-root target/release-config" in mac_job
    assert 'CAPSEM_ASSET_MANIFEST="$PREACTIVATION_MANIFEST"' in mac_job
    assert 'CAPSEM_CONFIG_ROOT="$PWD/target/release-config"' in mac_job
    assert 'CAPSEM_ASSETS_PATH="$PWD/target/release-assets"' in mac_job
    assert "CAPSEM_ARCH=arm64" in mac_job
    assert "bash scripts/materialize-config.sh" in mac_job
    assert mac_job.index("Fetch exact selected arm64 profiles") < mac_job.index(
        "bash scripts/materialize-config.sh"
    )
    assert '--manifest "$ASSET_MANIFEST_URL"' in mac_job
    assert "name: binary-channel-source" in linux_job
    assert "Fetch exact selected ${{ matrix.arch }} profiles" in linux_job
    assert "uses: ./.github/actions/fetch-release-inputs" in linux_job
    assert "architecture: ${{ matrix.arch }}" in linux_job
    assert "output: target/binary-selected-profiles" in linux_job
    assert "--input-dir target/binary-selected-profiles" in linux_job
    assert "--assets-dir target/package-content/assets" in linux_job
    assert "--config-root target/package-source-config" in linux_job
    assert 'CAPSEM_ASSET_MANIFEST="$PWD/target/package-content/assets/manifest.json"' in linux_job
    assert 'CAPSEM_CONFIG_ROOT="$PWD/target/package-source-config"' in linux_job
    assert 'CAPSEM_ASSETS_PATH="$PWD/target/package-content/assets"' in linux_job
    assert 'CAPSEM_CONFIG_OUTPUT_ROOT="$PWD/target/package-content/config"' in linux_job
    assert "bash scripts/materialize-config.sh --pair-content" in linux_job
    assert_unmasked_step(
        "release.yaml",
        yaml.safe_load(release),
        "build-app-linux",
        "Materialize runtime config",
    )
    assert 'CAPSEM_ARCH="${{ matrix.arch }}"' in linux_job
    assert "bash scripts/materialize-config.sh" in linux_job
    assert linux_job.index("Fetch exact selected ${{ matrix.arch }} profiles") < linux_job.index(
        "bash scripts/materialize-config.sh"
    )
    assert "uv run --project build_system --frozen capsem-gate cross-compile" in linux_job
    assert "--content-root target/package-content" in linux_job
    assert "--defer-proof" in linux_job
    assert "CAPSEM_INSTALL_MANIFEST_URL: https://release.capsem.org/assets/" in linux_job
    for mutable in ("sudo apt-get", "pnpm install", "cargo install", "cargo tauri build"):
        assert mutable not in linux_job
    assert "--profile config/profiles/code/profile.toml" not in release
    for assembler in (
        "build_system/packaging/macos/build-pkg.sh",
        "build_system/packaging/linux/repack-deb.sh",
    ):
        source = _source_text(assembler)
        assert 'for profile_path in "$CONFIG_ROOT"/profiles/*/profile.toml' in source
        assert 'profile validate "$profile_path"' in source
        assert '--config-root "$CONFIG_ROOT" --materialized' in source


def test_linux_release_always_retains_full_per_arch_gate_evidence() -> None:
    linux_job = _workflow_job_block("build-app-linux", "release.yaml")
    upload = linux_job.split("- name: Upload Linux package gate evidence", maxsplit=1)[1]

    assert "- arch: arm64" in linux_job
    assert "- arch: x86_64" in linux_job
    assert "if: always()" in upload
    assert "uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a" in upload
    assert "name: release-linux-${{ matrix.arch }}-gate-runs-${{ github.run_attempt }}" in upload
    assert "path: target/gate-runs/" in upload
    assert "if-no-files-found: error" in upload


def test_binary_pairing_failure_uploads_exported_prefix_evidence() -> None:
    job = _workflow_job("test-binary-pairing", "release.yaml")
    step = next(
        row for row in job["steps"] if row.get("name") == "Upload pairing evidence on failure"
    )

    assert step["if"] == "failure()"
    assert set(step["with"]["path"].splitlines()) >= {
        "target/test-artifacts/",
        "target/gate-runs/",
        "target/test-home/.capsem/run/sessions/",
        "target/test-home/.capsem/logs/",
    }
    assert step["with"]["include-hidden-files"] is True
    assert step["with"]["if-no-files-found"] == "error"


def test_hosted_install_failure_uploads_exact_gate_and_glowup_evidence() -> None:
    from capsem_builder.gate import config as gate_config

    job = _workflow_job("test-install")
    step = next(
        row
        for row in job["steps"]
        if row.get("name") == "Upload install and glow-up evidence on failure"
    )
    evidence = gate_config.load(PROJECT_ROOT).install.layout.glowup_evidence

    install = next(row for row in job["steps"] if row.get("name") == "Run install e2e tests")
    assert install["id"] == "install_e2e"
    assert step["if"] == "failure()"
    assert step["uses"] == "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
    assert set(step["with"]["path"].splitlines()) == {
        "target/gate-runs/",
        f"{evidence}/",
    }
    assert step["with"]["if-no-files-found"] == (
        "${{ steps.install_e2e.outcome == 'failure' && 'error' || 'warn' }}"
    )


def test_all_quick_session_entrypoints_preserve_profile_selection() -> None:
    app = _source_text("web/app/src/lib/components/shell/App.svelte")
    tray_main = _source_text("crates/capsem-tray/src/main.rs")
    tray_gateway = _source_text("crates/capsem-tray/src/gateway.rs")
    cli = _source_text("crates/capsem/src/main.rs")
    mcp = _source_text("crates/capsem-mcp/src/main.rs")

    assert "vmStore.openCreateModal()" in app
    assert "profile_id: 'code'" not in app
    new_session = tray_main.split("Action::NewSession =>", maxsplit=1)[1].split(
        "Action::Save", maxsplit=1
    )[0]
    assert "launch_ui(None)" in new_session
    assert "provision_temp" not in new_session
    assert "provision_temp" not in tray_gateway
    assert 'profile_id":"code' not in tray_gateway
    assert "profile_id: profile.clone()" in cli
    assert "params.profile.as_deref().unwrap_or(DEFAULT_PROFILE_ID)" in mcp


def test_just_test_runs_grep_guardrails_for_hardcoded_release_selections() -> None:
    canonical_gate = _dispatched_text("test-clean:")
    guard = _source_text("build_system/scripts/audit/check-hardcoded-release-selections.sh") + _source_text(
        "build_system/builder/gate/tools/audit/release_selections.py"
    )
    reusable_channel = _workflow_text("release-channel.yaml")

    assert "bash build_system/scripts/audit/check-hardcoded-release-selections.sh" in canonical_gate
    for term in ("code", "co-work", "cowork", "terminal", "termional", "gui"):
        assert term in guard
    assert "ripgrep" in guard
    assert "profile_id" in guard
    assert "--profile" in guard
    assert "stable" in guard
    assert "nightly" in guard
    assert "ASSET_MANIFEST_URL" in guard
    assert ".github/workflows" in guard
    assert 'glob("*/profile.toml")' in guard
    assert "builtin_profile_configs" in guard
    assert "unwrap_or" in guard
    assert "DEFAULT_RELEASE_MANIFEST_URL" in guard
    assert "channel:\n        type: string\n        required: true" in reusable_channel
    assert "inputs.channel || 'stable'" not in reusable_channel
    assert "CHANNEL: ${{ inputs.channel }}" in reusable_channel


def test_release_selection_match_guard_is_directly_unit_testable(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    source = tmp_path / "surface.py"
    source.write_text('profile_id: "code"\n', encoding="utf-8")

    assert release_selections.reject_matches(
        tmp_path,
        "fixture hardcodes a profile",
        r"profile_id:\s*['\"]code['\"]",
        ("surface.py",),
    )
    assert "fixture hardcodes a profile" in capsys.readouterr().err


def test_hardcoded_release_selection_guard_runs_without_ripgrep(tmp_path: Path) -> None:
    tool_bin = tmp_path / "bin"
    tool_bin.mkdir()
    for command in ("python3",):
        source = shutil.which(command)
        assert source is not None, f"test host is missing {command}"
        (tool_bin / command).symlink_to(source)

    assert shutil.which("rg", path=str(tool_bin)) is None
    result = subprocess.run(
        [
            "/bin/bash",
            str(
                PROJECT_ROOT
                / "build_system/scripts/audit/check-hardcoded-release-selections.sh"
            ),
        ],
        cwd=PROJECT_ROOT,
        env={
            **os.environ,
            "CAPSEM_GUARD_ROOT": str(PROJECT_ROOT),
            "PATH": str(tool_bin),
        },
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert "Hardcoded profile/channel selection guard passed." in result.stdout


def test_hardcoded_release_selection_guard_rejects_each_regression(tmp_path: Path) -> None:
    fixture_paths = (
        ".github/workflows",
        "config/profiles",
        "web/app/src/lib/components",
        "crates/capsem-tray/src",
        "crates/capsem-mcp/src/main.rs",
        "crates/capsem/src/main.rs",
        "crates/capsem/src/update.rs",
        "crates/capsem-service/src/main.rs",
        "crates/capsem-core/src/net/policy_config/profile_contract.rs",
        "build_system/packaging/macos/build-pkg.sh",
        "build_system/packaging/linux/repack-deb.sh",
        "build_system/packaging/linux/deb-postinst.sh",
        "build_system/packaging/macos/pkg-scripts/postinstall",
        "scripts/materialize-config.sh",
        "build_system/builder/release/tools/build_complete_release_channel.py",
        "build_system/builder/release/tools/local_release_glowup.py",
        "tests/capsem-install",
        "justfile",
    )
    for relative in fixture_paths:
        source = PROJECT_ROOT / relative
        target = tmp_path / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_dir():
            shutil.copytree(source, target)
        else:
            shutil.copy2(source, target)

    guard = (
        PROJECT_ROOT
        / "build_system/scripts/audit/check-hardcoded-release-selections.sh"
    )
    tool_bin = tmp_path / "tool-bin"
    tool_bin.mkdir()
    python3 = shutil.which("python3")
    assert python3 is not None
    (tool_bin / "python3").symlink_to(python3)
    assert shutil.which("rg", path=str(tool_bin)) is None
    env = {
        **os.environ,
        "CAPSEM_GUARD_ROOT": str(tmp_path),
        "PATH": str(tool_bin),
    }

    def run_guard() -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/bash", str(guard)],
            cwd=PROJECT_ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    baseline = run_guard()
    assert baseline.returncode == 0, baseline.stderr

    dialog = tmp_path / "web/app/src/lib/components/shell/CreateSandboxDialog.svelte"
    for profile in ("code", "co-work", "cowork", "terminal", "termional", "gui"):
        original = dialog.read_text()
        dialog.write_text(original + f"\n<!-- profile_id: '{profile}' -->\n")
        rejected = run_guard()
        dialog.write_text(original)
        assert rejected.returncode != 0, f"guard accepted hardcoded profile {profile}"
        assert "hardcodes a named profile" in rejected.stderr

    workflow = tmp_path / ".github/workflows/release-binary-staging.yaml"
    for channel in ("stable", "nightly"):
        original = workflow.read_text()
        workflow.write_text(
            original
            + f"\n# ASSET_MANIFEST_URL: https://release.capsem.org/assets/{channel}/manifest.json\n"
        )
        rejected = run_guard()
        workflow.write_text(original)
        assert rejected.returncode != 0, f"guard accepted hardcoded channel {channel}"
        assert "hardcodes a stable/nightly ASSET_MANIFEST_URL" in rejected.stderr

    postinstall = tmp_path / "build_system/packaging/linux/deb-postinst.sh"
    original = postinstall.read_text()
    postinstall.write_text(
        original + "\n# MANIFEST_SOURCE='https://release.capsem.org/assets/nightly/manifest.json'\n"
    )
    rejected = run_guard()
    postinstall.write_text(original)
    assert rejected.returncode != 0
    assert "postinstall silently falls back" in rejected.stderr

    update = tmp_path / "crates/capsem/src/update.rs"
    original = update.read_text()
    update.write_text(original + "\n// value.unwrap_or(DEFAULT_RELEASE_MANIFEST_URL)\n")
    rejected = run_guard()
    update.write_text(original)
    assert rejected.returncode != 0
    assert "installed update flow silently substitutes" in rejected.stderr

    future_profile = tmp_path / "config/profiles/terminal/profile.toml"
    future_profile.parent.mkdir(parents=True)
    shutil.copy2(tmp_path / "config/profiles/code/profile.toml", future_profile)
    rejected = run_guard()
    assert rejected.returncode != 0
    assert "builtin_profile_configs does not exactly mirror" in rejected.stderr
    future_profile.unlink()

    for future_name in ("terminal", "gui"):
        future_profile = tmp_path / f"config/profiles/{future_name}/profile.toml"
        future_profile.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(tmp_path / "config/profiles/code/profile.toml", future_profile)
        rejected = run_guard()
        future_profile.unlink()
        assert rejected.returncode != 0, f"guard accepted unembedded profile {future_name}"
        assert "builtin_profile_configs does not exactly mirror" in rejected.stderr

    profile_contract = tmp_path / "crates/capsem-core/src/net/policy_config/profile_contract.rs"
    original = profile_contract.read_text()
    profile_contract.write_text(
        original + '\n// include_str!("../../../../../config/profiles/gui/profile.toml")\n'
    )
    rejected = run_guard()
    profile_contract.write_text(original)
    assert rejected.returncode != 0
    assert "builtin_profile_configs does not exactly mirror" in rejected.stderr

    original = dialog.read_text()
    for regression in ("profileId = 'terminal'", "<option value='gui'>GUI</option>"):
        dialog.write_text(original + f"\n<!-- {regression} -->\n")
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted picker regression {regression}"
        assert "profile picker fabricates" in rejected.stderr
    dialog.write_text(original)

    mcp = tmp_path / "crates/capsem-mcp/src/main.rs"
    original = mcp.read_text()
    for regression, message in (
        ('// "profile_id": DEFAULT_PROFILE_ID\n', "MCP request bypasses"),
        ('// "/profiles/{}/mcp/servers", DEFAULT_PROFILE_ID\n', "silently uses the default"),
    ):
        mcp.write_text(original + "\n" + regression)
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted {regression.strip()}"
        assert message in rejected.stderr
    mcp.write_text(original)

    release_workflow = tmp_path / ".github/workflows/release.yaml"
    original = release_workflow.read_text()
    release_workflow.write_text(original + "\n# --profile config/profiles/gui/profile.toml\n")
    rejected = run_guard()
    release_workflow.write_text(original)
    assert rejected.returncode != 0
    assert "materializes one named profile" in rejected.stderr

    for selection in (
        "stable",
        "nightly",
        "code",
        "co-work",
        "cowork",
        "terminal",
        "termional",
        "gui",
    ):
        original = workflow.read_text()
        input_name = "channel" if selection in {"stable", "nightly"} else "profile"
        workflow.write_text(
            original + f"\nregression:\n  {input_name}:\n    default: {selection}\n"
        )
        rejected = run_guard()
        workflow.write_text(original)
        assert rejected.returncode != 0, f"guard accepted workflow default {selection}"
        assert "silently defaults" in rejected.stderr

    installed_update_test = tmp_path / "tests/capsem-install/test_update.py"
    original_installed_update_test = installed_update_test.read_text()
    for override in ("CAPSEM_RELEASE_MANIFEST_URL", "CAPSEM_RELEASE_HEALTH_URL"):
        installed_update_test.write_text(
            original_installed_update_test + f'\n# "{override}": manifest_url\n'
        )
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted installed test override {override}"
        assert "installed update test bypasses manifest-metadata" in rejected.stderr
    installed_update_test.write_text(original_installed_update_test)

    reusable = tmp_path / ".github/workflows/release-channel.yaml"
    original = reusable.read_text()
    reusable.write_text(original + "\nchannel:\n  type: string\n  required: false\n")
    rejected = run_guard()
    assert rejected.returncode != 0
    assert "makes its channel optional" in rejected.stderr
    reusable.write_text(original + "\n# ${{ inputs.channel || 'stable' }}\n")
    rejected = run_guard()
    reusable.write_text(original)
    assert rejected.returncode != 0
    assert "silently substitutes stable" in rejected.stderr

    doctrine = tmp_path / "docs/regression.md"
    doctrine.parent.mkdir(exist_ok=True)
    retired_markers = (
        "release-" + "qualification.yaml",
        "check-release-" + "qualification.py",
        "qualify-" + "release",
        "cut-" + "release",
    )
    for marker in retired_markers:
        doctrine.write_text(f"{marker}\n")
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted retired marker {marker}"
        assert "retired independent release doctrine" in rejected.stderr
    doctrine.unlink()

    changelog = tmp_path / "CHANGELOG.md"
    changelog.write_text("\n".join(retired_markers) + "\n")
    historical = run_guard()
    changelog.unlink()
    assert historical.returncode == 0, historical.stderr

    profile_workflow = tmp_path / ".github/workflows/release-assets.yaml"
    original_profile_workflow = profile_workflow.read_text()
    profile_workflow.write_text(
        original_profile_workflow.replace(
            "group: capsem-release-${{ inputs.channel }}",
            "group: capsem-profile-${{ inputs.channel }}",
            1,
        )
    )
    rejected = run_guard()
    profile_workflow.write_text(original_profile_workflow)
    assert rejected.returncode != 0
    assert "shared per-channel release lock" in rejected.stderr

    rogue_writer = tmp_path / ".github/workflows/rogue-writer.yaml"
    rogue_writer.write_text("steps:\n  - run: python scripts/stage-profile-publication.py\n")
    rejected = run_guard()
    rogue_writer.unlink()
    assert rejected.returncode != 0
    assert "source-manifest writer outside serialized release workflows" in rejected.stderr

    rogue_deploy = tmp_path / ".github/workflows/rogue-deploy.yaml"
    rogue_deploy.write_text(
        "jobs:\n  deploy:\n    uses: ./.github/workflows/release-channel.yaml\n"
    )
    rejected = run_guard()
    rogue_deploy.unlink()
    assert rejected.returncode != 0
    assert "production deploy caller outside serialized release workflows" in rejected.stderr

    for relative in (
        "build_system/packaging/linux/deb-postinst.sh",
        "build_system/packaging/macos/pkg-scripts/postinstall",
    ):
        postinstall = tmp_path / relative
        original = postinstall.read_text()
        for channel in ("stable", "nightly"):
            postinstall.write_text(
                original
                + f"\n# MANIFEST_SOURCE='https://release.capsem.org/assets/{channel}/manifest.json'\n"
            )
            rejected = run_guard()
            assert rejected.returncode != 0, f"guard accepted {relative} fallback {channel}"
            assert "postinstall silently falls back" in rejected.stderr
        postinstall.write_text(
            original
            + "\n# CAPSEM_RELEASE_MANIFEST_URL=https://release.capsem.org/assets/stable/manifest.json\n"
        )
        rejected = run_guard()
        assert rejected.returncode != 0
        assert "postinstall bypasses installed manifest-metadata" in rejected.stderr
        postinstall.write_text(original)

    original = update.read_text()
    for fallback in (
        "value.unwrap_or(DEFAULT_RELEASE_MANIFEST_URL)",
        "value.unwrap_or_else(|| DEFAULT_RELEASE_MANIFEST_URL)",
    ):
        update.write_text(original + f"\n// {fallback}\n")
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted update fallback {fallback}"
        assert "installed update flow silently substitutes" in rejected.stderr
    update.write_text(original)

    for legacy_sidecar in ("update-check.json", "manifest-origin.json"):
        update.write_text(original + f'\n// "{legacy_sidecar}"\n')
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted legacy {legacy_sidecar}"
        assert "legacy split manifest/update sidecar" in rejected.stderr
    update.write_text(original)

    release_reader = (
        tmp_path / "build_system/builder/release/tools/build_complete_release_channel.py"
    )
    original_reader = release_reader.read_text()
    release_reader.write_text(original_reader + "\n# urllib.request.urlopen(url, timeout=60)\n")
    rejected = run_guard()
    release_reader.write_text(original_reader)
    assert rejected.returncode != 0
    assert "public release HTTP reader passes a bare URL" in rejected.stderr


def test_pr_ci_python_coverage_is_not_a_monolithic_vm_tree_rerun() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    coverage_step = workflow.split(
        "- name: Cross-system Python schema tests with coverage", maxsplit=1
    )[1].split(
        "# Python integration tests that need no VM",
        maxsplit=1,
    )[0]

    assert "pytest tests/ --cov" not in coverage_step
    assert "tests/capsem-install" not in coverage_step
    assert "tests/capsem-serial" not in coverage_step
    assert "tests/ironbank" not in coverage_step
    assert "tests/capsem-mcp" not in coverage_step
    assert "tests/capsem-service" not in coverage_step
    assert "build_system/tests/image/test_config.py" in coverage_step
    assert "build_system/tests/image/test_manifest.py" in coverage_step
    assert "build_system/tests/image/test_models.py" in coverage_step
    assert "build_system/tests/image/test_skills.py" in coverage_step
    assert "--cov=build_system/builder" in coverage_step
    assert "--cov=src/capsem" not in coverage_step


def test_generate_settings_creates_catalog_directory_before_redirect() -> None:
    script = (PROJECT_ROOT / "scripts" / "generate-settings.sh").read_text()

    mkdir_pos = script.find('mkdir -p "$ROOT/target/config/profiles"')
    catalog_pos = script.find("target/config/profiles/catalog.generated.json")

    assert mkdir_pos != -1
    assert catalog_pos != -1
    assert mkdir_pos < catalog_pos


def test_live_provider_dotenv_files_are_gitignored() -> None:
    for name in [".env", ".env.local", ".env.ironbank"]:
        ignored = subprocess.run(
            ["git", "check-ignore", "-q", name],
            cwd=PROJECT_ROOT,
            check=False,
        )
        assert ignored.returncode == 0, f"{name} must be gitignored before live canaries"


def test_pr_ci_non_vm_python_tests_prepare_assets_and_signed_binaries() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    block = workflow.split("- name: Python integration tests (non-VM suites)", maxsplit=1)[1].split(
        "# Verify all integration test suites", maxsplit=1
    )[0]

    asset_pos = block.find("bash scripts/prepare-install-test-assets.sh")
    build_pos = block.find(
        "cargo build -p capsem-process -p capsem-service -p capsem -p capsem-mcp"
    )
    bench_package_pos = block.find("-p capsem-bench")
    bench_binary_pos = block.find("target/debug/capsem-bench-rs")
    sign_pos = block.find("codesign --sign - --entitlements build_system/packaging/macos/entitlements.plist --force")
    pytest_pos = block.find("uv run --project build_system --frozen python -m pytest -c build_system/pyproject.toml tests/capsem-bootstrap/")

    assert asset_pos != -1
    assert build_pos != -1
    assert bench_package_pos != -1
    assert bench_binary_pos != -1
    assert "target/debug/capsem-bench;" not in block
    assert sign_pos != -1
    assert pytest_pos != -1
    assert asset_pos < pytest_pos
    assert build_pos < pytest_pos
    assert bench_package_pos < bench_binary_pos
    assert sign_pos < pytest_pos


def test_kvm_checkpoint_x86_state_tests_are_arch_gated() -> None:
    tests = (
        PROJECT_ROOT
        / "crates"
        / "capsem-core"
        / "src"
        / "hypervisor"
        / "kvm"
        / "checkpoint"
        / "tests.rs"
    ).read_text()

    assert "fn test_header() -> CheckpointHeader" in tests
    assert "let header = test_header();" in tests
    assert "CheckpointHeader::current" not in tests

    x86_symbols = [
        "fn snapshot(",
        "fn vm_snapshot()",
        "fn mmio(",
        "fn writes_header_and_memory()",
        "fn restores_memory_and_vcpu_state()",
        "fn overwrites_atomically()",
        "fn rejects_missing_parent()",
        "fn removes_temp_file_after_create_failure()",
        "fn restore_rejects_wrong_ram_size()",
        "fn restore_rejects_wrong_vcpu_count()",
        "fn restore_rejects_trailing_bytes()",
    ]
    for symbol in x86_symbols:
        prefix = tests.split(symbol, maxsplit=1)[0].rsplit("\n", maxsplit=4)[0]
        window = tests[len(prefix) : tests.find(symbol)]
        assert '#[cfg(target_arch = "x86_64")]' in window


def test_mock_server_uses_rust_fixture_crate() -> None:
    root_cargo = (PROJECT_ROOT / "Cargo.toml").read_text()
    cli_cargo = (PROJECT_ROOT / "crates" / "capsem" / "Cargo.toml").read_text()
    cli_main = (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()

    assert '"crates/capsem-mock-server"' in root_cargo
    assert "capsem-mock-server" not in cli_cargo
    assert "mock_server_impl" not in cli_main
    assert "capsem-mock-server" in cli_main


def test_serial_benchmark_release_proofs_are_not_env_gated() -> None:
    benchmark = PROJECT_ROOT / "tests" / "capsem-serial" / "test_mock_server_protocol_benchmark.py"
    source = benchmark.read_text()

    assert "CAPSEM_RUN_MOCK_SERVER_PROTOCOL_BENCH" not in source
    assert "pytest.skip(" not in source
    assert "total_requests = 10" not in source
    assert 'CAPSEM_BENCH_TOTAL_REQUESTS", "10"' not in source
    assert 'CAPSEM_BENCH_CONCURRENCY", "1"' not in source
    assert '"capsem-bench-rs",' in source
    assert '"protocol",' in source


def test_benchmark_release_path_wires_mock_server_and_forbids_http_skip() -> None:
    candidate = _recipe_block("_test-candidate:")
    baseline = (
        PROJECT_ROOT / "tests" / "capsem-serial" / "test_capsem_bench_baseline.py"
    ).read_text()

    assert "tests/capsem-serial/test_capsem_bench_baseline.py" in candidate
    assert '{{cli_binary}} run "capsem-bench"' not in candidate
    assert "from helpers.mock_server import start_mock_server, stop_process" in baseline
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in baseline
    assert "CAPSEM_MOCK_SERVER_HTTPS_BASE_URL" in baseline
    assert "CAPSEM_BENCH_TOTAL_REQUESTS" in baseline
    assert "CAPSEM_BENCH_CONCURRENCY" in baseline
    assert "RELEASE_PROTOCOL_REQUESTS = 50_000" in baseline
    assert "RELEASE_PROTOCOL_CONCURRENCY = 64" in baseline
    assert "RELEASE_PROTOCOL_REQUESTS = 10" not in baseline
    assert "RELEASE_PROTOCOL_CONCURRENCY = 1" not in baseline
    assert "validate_capsem_bench_result(data)" in baseline
    assert "capsem-bench all" in baseline
    assert "skipped" in baseline
    assert 'benchmark_output_dir(PROJECT_ROOT, "capsem-bench")' in baseline


def test_integration_script_has_no_live_ai_provider_escape_hatch() -> None:
    source = (PROJECT_ROOT / "scripts" / "integration_test.py").read_text()

    assert "GEMINI_API_KEY" not in source
    assert "GOOGLE_API_KEY" not in source
    assert "googleapis.com" not in source
    assert "include_gemini_probe" not in source


def test_integration_script_uses_current_tool_call_arguments_column() -> None:
    source = (PROJECT_ROOT / "scripts" / "integration_test.py").read_text()

    assert "request_preview FROM tool_calls" not in source
    assert "SELECT id, arguments FROM tool_calls WHERE origin = 'mcp'" in source


def test_builder_has_no_legacy_ai_provider_authoring_rail() -> None:
    forbidden = (
        "AiProviderConfig",
        "ApiKeyConfig",
        "add_ai_provider",
        "include_providers",
        "ai_providers",
        "config/ai",
        'config" / "ai"',
        "AI provider",
    )
    checked_roots = [
        PROJECT_ROOT / "src" / "capsem" / "builder",
        PROJECT_ROOT / "guest" / "config",
    ]
    offenders: list[str] = []
    for root in checked_roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            if path == Path(__file__) or path.name == "test_active_docs_profile_contract.py":
                continue
            rel = path.relative_to(PROJECT_ROOT)
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for marker in forbidden:
                if marker in text:
                    offenders.append(f"{rel}: contains {marker!r}")
                    break

    assert offenders == [], "legacy AI-provider builder rail still exists:\n" + "\n".join(offenders)


def test_gateway_docs_describe_explicit_routes_not_generic_forwarding() -> None:
    docs = "\n".join(
        [
            _source_text("docs/src/content/docs/architecture/service-api.md"),
            _skill_text("skills/site-architecture/SKILL.md"),
            _source_text("skills/frontend-design/SKILL.md"),
        ]
    )

    assert "Unknown routes must return 404" in docs
    assert "explicit route table" in docs
    assert "`*` (fallback)" not in docs
    assert "transparent fallback" not in docs
    assert "Transparent proxy" not in docs
    assert "transparently" not in docs
    assert "generic path forwarding" not in docs


def test_config_contract_has_no_admin_or_registry_authority() -> None:
    assert not (PROJECT_ROOT / "config" / "admin").exists()
    assert (PROJECT_ROOT / "config" / "settings" / "settings.toml").is_file()
    assert (PROJECT_ROOT / "config" / "settings" / "schema.generated.json").is_file()
    assert (PROJECT_ROOT / "config" / "settings" / "ui-metadata.toml").is_file()
    assert (PROJECT_ROOT / "config" / "settings" / "ui-metadata.generated.json").is_file()

    forbidden = (
        "config/admin",
        "config/guest",
        "settings registry",
        "settings-registry",
        "settings-schema.generated",
        "mcp-tools.generated",
    )
    checked_roots = [
        PROJECT_ROOT / "scripts",
        PROJECT_ROOT / "src" / "capsem" / "builder",
        PROJECT_ROOT / "crates" / "capsem-admin" / "src",
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "net" / "policy_config",
        PROJECT_ROOT / "tests",
        PROJECT_ROOT / "docs" / "src" / "content" / "docs",
        PROJECT_ROOT / "skills",
        PROJECT_ROOT / ".github" / "workflows",
    ]
    offenders: list[str] = []
    for root in checked_roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            if path == Path(__file__) or path.name == "test_active_docs_profile_contract.py":
                continue
            rel = path.relative_to(PROJECT_ROOT)
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for marker in forbidden:
                if marker in text:
                    offenders.append(f"{rel}: contains {marker!r}")
                    break
    assert offenders == [], "admin/registry config authority still exists:\n" + "\n".join(offenders)


def test_builder_has_no_guest_scaffold_authoring_rail() -> None:
    assert not (PROJECT_ROOT / "src" / "capsem" / "builder" / "scaffold.py").exists()
    assert not (PROJECT_ROOT / "tests" / "test_scaffold.py").exists()

    forbidden = (
        "capsem-builder init",
        "capsem-builder new",
        "capsem-builder add",
        "capsem-builder mcp",
        "builder.scaffold",
        "scaffold.py",
        "init_guest_dir",
        "new_image",
        "scan_base_config",
        "add_package_set",
        "add_mcp_server",
    )
    checked_roots = [
        PROJECT_ROOT / "src" / "capsem" / "builder",
        PROJECT_ROOT / "docs" / "src" / "content" / "docs",
        PROJECT_ROOT / "skills",
        PROJECT_ROOT / ".github" / "workflows",
    ]
    offenders: list[str] = []
    for root in checked_roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            rel = path.relative_to(PROJECT_ROOT)
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for marker in forbidden:
                if marker in text:
                    offenders.append(f"{rel}: contains {marker!r}")
                    break
    assert offenders == [], "builder scaffold rail still exists:\n" + "\n".join(offenders)


def test_guest_init_exports_ca_bundle_for_runtime_and_login_shells() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
    expected = {
        "SSL_CERT_FILE": "/etc/ssl/certs/ca-certificates.crt",
        "REQUESTS_CA_BUNDLE": "/etc/ssl/certs/ca-certificates.crt",
        "NODE_EXTRA_CA_CERTS": "/etc/ssl/certs/ca-certificates.crt",
    }

    runtime_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[0]
    profile_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[1]

    for key, value in expected.items():
        export = f"export {key}={value}"
        assert export in runtime_block
        assert export in profile_block


def test_guest_init_exports_terminal_type_for_exec_and_doctor() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
    runtime_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[0]
    profile_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[1]

    assert "export TERM=xterm-256color" in runtime_block
    assert "export TERM=xterm-256color" in profile_block


def test_guest_init_repairs_overlay_root_traversal_for_unprivileged_tools() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    chmod_pos = init.find("chmod 755 /newroot")
    chroot_chmod_pos = init.find("chroot /newroot /bin/chmod 755 /")
    launch_pos = init.find('chroot /newroot "$AGENT_PATH"')

    assert chmod_pos != -1, "init must make / traversable for _apt and tool users"
    assert chroot_chmod_pos != -1, "init must repair root mode as seen inside chroot"
    assert launch_pos != -1
    assert chmod_pos < launch_pos
    assert chroot_chmod_pos < launch_pos


def test_guest_init_console_redirection_cannot_kill_pid_one() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    mknod_pos = init.find("mknod -m 600 /dev/console c 5 1")
    probe_pos = init.find('( : <"$candidate" >"$candidate" )')
    guarded_exec_pos = init.find('if [ -n "$CONSOLE_DEV" ]; then')
    fatal_exec = "exec 0</dev/console 1>/dev/console 2>/dev/console"

    assert mknod_pos != -1, "init must create /dev/console when devtmpfs omits it"
    assert probe_pos != -1, "init must preflight console opens before redirecting PID 1"
    assert guarded_exec_pos != -1, "init must guard console redirection with a usable device check"
    assert fatal_exec not in init, "hard /dev/console redirection exits PID 1 on KVM boot races"
    assert mknod_pos < probe_pos < guarded_exec_pos


def test_guest_init_persists_boot_diagnostics_before_agent_launch() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    helper_pos = init.find("init_log()")
    workspace_log_pos = init.find("/mnt/shared/workspace/.capsem-boot.log")
    agent_stdio_pos = init.find("/mnt/shared/workspace/.capsem-agent-stdio.log")
    backtrace_pos = init.find("export RUST_BACKTRACE=1")
    kmsg_pos = init.find("> /dev/kmsg")
    launch_log_pos = init.find('init_log "starting PTY agent (vsock mode): $AGENT_PATH"')
    chroot_pos = init.find('chroot /newroot "$AGENT_PATH" >> "$AGENT_STDIO_LOG" 2>&1')
    launch_pos = init.find('chroot /newroot "$AGENT_PATH"')
    exit_status_pos = init.find('init_log "PTY agent exited with status $AGENT_STATUS"')

    assert helper_pos != -1, "init must centralize durable boot diagnostics"
    assert workspace_log_pos != -1, "init diagnostics must survive in host-preserved workspace"
    assert agent_stdio_pos != -1, "agent stderr must survive when it exits before opening its log"
    assert backtrace_pos != -1, "early agent panics must include enough context to fix"
    assert kmsg_pos != -1, "init diagnostics must reach serial-visible kernel log on quiet boots"
    assert launch_log_pos != -1, "init must mark the exact agent launch boundary"
    assert chroot_pos != -1
    assert launch_pos != -1
    assert exit_status_pos != -1, (
        "init must report early agent exits instead of silently idling PID 1"
    )
    assert (
        helper_pos
        < workspace_log_pos
        < agent_stdio_pos
        < launch_log_pos
        < launch_pos
        < exit_status_pos
    )


def test_guest_init_publishes_rootfs_binaries_into_run_contract() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    expected_rootfs_copies = {
        "capsem-net-proxy": "/newroot/usr/local/bin/capsem-net-proxy",
        "capsem-dns-proxy": "/newroot/usr/local/bin/capsem-dns-proxy",
        "capsem-pty-agent": "/newroot/usr/local/bin/capsem-pty-agent",
        "capsem-sysutil": "/newroot/usr/local/bin/capsem-sysutil",
    }
    for binary, rootfs_path in expected_rootfs_copies.items():
        assert rootfs_path in init
        assert f"cp {rootfs_path} /newroot/run/{binary}" in init
        assert f"chmod 555 /newroot/run/{binary}" in init

    assert "ln -sf /run/capsem-sysutil /newroot/sbin/shutdown" not in init
    assert "rm -f /newroot/sbin/shutdown" in init

    for link in (
        "/newroot/sbin/halt",
        "/newroot/sbin/poweroff",
        "/newroot/sbin/reboot",
        "/newroot/usr/local/bin/suspend",
    ):
        assert f"ln -sf /run/capsem-sysutil {link}" in init


def test_guest_runtime_doctor_package_probes_are_hermetic() -> None:
    source = (PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_runtimes.py").read_text()

    forbidden_fragments = [
        "pip install six",
        "uv pip install wheel",
        "uv pip install humanize",
        "npm install -g cowsay",
        "npm install lodash",
        "apt-get install -y -qq htop",
    ]
    for fragment in forbidden_fragments:
        assert fragment not in source

    assert "--no-index" in source
    assert "file:" in source
    assert "dpkg-deb --build" in source
    assert "--python /root/.venv/bin/python" in source


def test_capsem_init_keeps_default_venv_out_of_workspace() -> None:
    """The boot venv must not become forked /root workspace state."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    assert "ln -sfn /run/capsem-venv /newroot/root/.venv" in init
    assert "uv venv --system-site-packages /run/capsem-venv" in init
    assert "python3 -m venv --system-site-packages /run/capsem-venv" in init
    assert "uv venv --system-site-packages /root/.venv" not in init
    assert "python3 -m venv --system-site-packages /root/.venv" not in init


def test_boot_timing_gate_attributes_regressions_to_one_stage() -> None:
    """Shared-runner jitter must not masquerade as a product boot regression."""
    module = _boot_timing_module()
    doctor_source = (
        PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_environment.py"
    ).read_text()
    jittered_stages = [
        {"name": "erofs", "duration_ms": 30},
        {"name": "virtiofs", "duration_ms": 30},
        {"name": "overlayfs", "duration_ms": 60},
        {"name": "workspace", "duration_ms": 60},
        {"name": "profile_root_seed", "duration_ms": 210},
        {"name": "network", "duration_ms": 270},
        {"name": "net_proxy", "duration_ms": 120},
        {"name": "dns_proxy", "duration_ms": 130},
        {"name": "deploy", "duration_ms": 70},
        {"name": "audit", "duration_ms": 140},
        {"name": "venv", "duration_ms": 10},
        {"name": "agent_start", "duration_ms": 10},
    ]

    assessment = module.assess_boot_timing(jittered_stages)
    assert assessment.total_ms == 1140
    assert assessment.slow_stages == ()

    regressed = module.assess_boot_timing(
        [{"name": "network", "duration_ms": module.MAX_BOOT_STAGE_MS + 10}]
    )
    assert regressed.slow_stages == (
        {"name": "network", "duration_ms": module.MAX_BOOT_STAGE_MS + 10},
    )
    assert "assessment = assess_boot_timing(stages)" in doctor_source
    assert "assert stages" in doctor_source
    assert "assert not assessment.slow_stages" in doctor_source
    assert "total <= 1000" not in doctor_source


def test_capsem_agent_repairs_missing_default_venv() -> None:
    """The guest agent must not leave VIRTUAL_ENV unset if init venv races."""
    source = (PROJECT_ROOT / "crates" / "capsem-agent" / "src" / "main.rs").read_text()

    assert 'const VENV_TARGET: &str = "/run/capsem-venv"' in source
    assert "std::thread::spawn(move ||" in source
    assert "std::fs::remove_dir_all(VENV_TARGET)" in source
    assert '.args(["venv", "--system-site-packages", VENV_TARGET])' in source
    assert '.args(["-m", "venv", "--system-site-packages", VENV_TARGET])' in source
    assert 'boot_env.push(("VIRTUAL_ENV".into(), VENV_DIR.into()))' in source
    assert "venv missing after init wait; creating fallback" in source
    assert "venv activated in boot_env" not in source


def test_suspend_snapshot_freezes_ext4_upper_before_ack_and_thaws_first_on_restore() -> None:
    """KVM checkpoints must not race writes into the persistent ext4 upper."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
    source = (PROJECT_ROOT / "crates" / "capsem-agent" / "src" / "main.rs").read_text()

    # `/` is overlayfs and does not implement FIFREEZE. Expose its ext4 upper
    # through the independently mounted devtmpfs so the agent can freeze the
    # filesystem without creating a bind-mount cycle inside the upperdir.
    assert "mkdir -p /newroot/dev/.capsem-system" in init
    assert "mount --bind /mnt/system /newroot/dev/.capsem-system" in init
    assert 'const SYSTEM_FS_MOUNT: &str = "/dev/.capsem-system";' in source

    prepare = source.split("Ok(HostToGuest::PrepareSnapshot) => {", maxsplit=1)[1].split(
        "Ok(HostToGuest::Unfreeze) => {", maxsplit=1
    )[0]
    assert "freeze_system_filesystem()" in prepare
    assert prepare.index("freeze_system_filesystem()") < prepare.index("GuestToHost::SnapshotReady")
    assert (
        "continue;"
        in prepare.split("freeze_system_filesystem()", maxsplit=1)[1].split(
            "GuestToHost::SnapshotReady", maxsplit=1
        )[0]
    )

    reconnect = source.split('eprintln!("[capsem-agent] reconnected successfully")', maxsplit=1)[
        1
    ].split("// Send BootReady", maxsplit=1)[0]
    assert reconnect.index("thaw_system_filesystem()") < reconnect.index(
        "rebind_workspace_after_resume()"
    )


def test_fork_route_flushes_without_thaw_before_clone() -> None:
    """Pre-fork quiescence must not pay fsfreeze cost and thaw before clone."""
    source = (PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs").read_text()
    fork_block = source.split("async fn handle_fork", maxsplit=1)[1].split(
        "Ok(Json(ForkResponse", maxsplit=1
    )[0]

    assert 'command: "sync; true".to_string()' in fork_block
    assert 'command: "fsfreeze' not in fork_block


def test_linux_vm_launch_preformats_system_overlay_before_boot() -> None:
    """Doctor boot must not pay first-boot mke2fs cost inside the guest."""
    core = (PROJECT_ROOT / "crates" / "capsem-core" / "src" / "lib.rs").read_text()
    process = (PROJECT_ROOT / "crates" / "capsem-process" / "src" / "main.rs").read_text()
    service = (PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs").read_text()
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    assert "pub fn preformat_system_overlay_image_if_needed" in core
    assert "pub fn ensure_preformatted_system_overlay_template" in core
    assert "pub fn preformat_system_overlay_image_from_template_if_needed" in core
    assert "auto_snapshot::clone_file(template_path, &tmp_path)" in core
    assert "system_overlay_has_ext4_magic(path)?" in core
    assert "lazy_itable_init=1,lazy_journal_init=1" in core
    assert '.arg("size=4")' in core
    assert "preformat_system_overlay_image_from_template_if_needed" in process
    assert "system_overlay_template_path_for_session" in process
    assert "session_dir, scratch_disk_size_gb" in process
    assert "fn prewarm_system_overlay_templates" in service
    assert "ensure_preformatted_system_overlay_template(&template_path, size_gb)" in service
    assert "prewarm_system_overlay_templates(&run_dir, &profile_cache)" in service
    assert "fn prewarm_vm_asset_hash_cache" in service
    assert "capsem_core::VmConfig::verify_hash(path, hash)" in service
    assert (
        "prewarm_vm_asset_hash_cache(&assets_base_dir, manifest.as_deref(), &current_version)"
        in service
    )
    assert "mke2fs unavailable; guest will format system overlay at first boot" in process
    assert "lazy_itable_init=1,lazy_journal_init=1" in init
    assert "-J size=4" in init


def test_raw_guest_vsock_probes_resolve_kvm_port_offset() -> None:
    """Raw guest vsock probes must connect to logical ports on KVM."""
    source = (PROJECT_ROOT / "tests" / "capsem-e2e" / "test_framed_mcp_mitm.py").read_text()

    assert "def capsem_vsock_port(logical_port):" in source
    assert "capsem.vsock_port_offset=" in source
    assert "VMADDR_CID_HOST, capsem_vsock_port(5002)" in source
    assert "VMADDR_CID_HOST, capsem_vsock_port(5003)" in source
    assert "VMADDR_CID_HOST, 5002" not in source
    assert "VMADDR_CID_HOST, 5003" not in source


def test_create_route_does_not_wait_for_full_guest_readiness() -> None:
    """Create catches immediate boot crashes; exec/file routes own readiness waits."""
    source = (PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs").read_text()
    provision_attempt = source.split("async fn provision_attempt", maxsplit=1)[1].split(
        "\n#[cfg(unix)]", maxsplit=1
    )[0]

    assert "std::time::Duration::from_millis(500)" in provision_attempt
    assert "exec/file routes own the full readiness wait" in provision_attempt
    assert "std::time::Duration::from_secs(5)" not in provision_attempt


def test_guest_runtime_doctor_apt_https_trust_probe_is_hermetic_release_gate() -> None:
    """Doctor must catch apt HTTPS/CA breakage without a mutable public mirror."""
    source = (PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_runtimes.py").read_text()

    assert "def test_apt_https_trust_is_readable_by_sandbox_user" in source
    assert "runuser -u _apt -- test -r /etc/ssl/certs/ca-certificates.crt" in source
    assert "/etc/apt/sources.list.d 2>/dev/null || true" in source
    assert "/etc/apt/sources.list 2>/dev/null || true" in source
    assert ") | \"\n        \"grep -F 'https://deb.debian.org'" in source
    assert "/etc/apt/sources.list /etc/apt/sources.list.d" not in source
    assert "def _bounded_remote_apt" not in source
    assert "apt-get update" not in source


def test_capsem_init_recreates_user_local_ai_cli_shims() -> None:
    """Curl-installed AI CLIs must keep the user-local shim expected by doctors."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    assert "for cli in claude agy; do" in init
    assert 'ln -sf "/opt/ai-clis/bin/$cli" "/newroot/usr/local/bin/$cli"' in init
    assert 'rm -f "/newroot/root/.local/bin/$cli"' in init
    assert 'ln -sf "/usr/local/bin/$cli" "/newroot/root/.local/bin/$cli"' in init
    assert 'chroot /newroot /bin/chmod 555 "/root/.local/bin/$cli"' in init


def test_capsem_init_keeps_etc_traversable_for_apt_sandbox() -> None:
    """The `_apt` sandbox must be able to read the TLS trust bundle under /etc."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    profile_seed_pos = init.find("projecting profile root seed")
    final_etc_chmod_pos = init.rfind("chmod 755 /newroot/etc")
    launch_pos = init.find('chroot /newroot "$AGENT_PATH"')

    assert "chmod 755 /newroot" in init
    assert profile_seed_pos != -1
    assert final_etc_chmod_pos != -1
    assert launch_pos != -1
    assert profile_seed_pos < final_etc_chmod_pos < launch_pos
    assert "TLS trust lives under `/etc/ssl/certs`" in init


def test_profile_roots_do_not_force_local_or_mock_model_providers() -> None:
    """Checked-in profile seeds must not silently select local/test model providers."""
    forbidden_fragments = (
        "127.0.0.1:11434",
        "localhost:11434",
        "CAPSEM_MOCK_SERVER",
        '"provider": "ollama"',
        '"baseUrl": "http://127.0.0.1:11434"',
    )
    for profile_dir in sorted((PROJECT_ROOT / "config" / "profiles").iterdir()):
        if not profile_dir.is_dir():
            continue
        config_path = profile_dir / "root" / "root" / ".codex" / "config.toml"
        if not config_path.exists():
            continue
        config = tomllib.loads(config_path.read_text())
        assert config.get("model_provider") not in {"local_ollama", "ollama"}, (
            f"{config_path} must not force a local Ollama model provider"
        )
        providers = config.get("model_providers") or {}
        assert "local_ollama" not in providers, (
            f"{config_path} must not declare a hidden local_ollama provider"
        )
        assert "ollama" not in providers, f"{config_path} must not declare a hidden ollama provider"
        root_dir = profile_dir / "root"
        for payload in sorted(root_dir.rglob("*")):
            if not payload.is_file():
                continue
            text = payload.read_text(errors="ignore")
            for fragment in forbidden_fragments:
                assert fragment not in text, f"{payload} contains {fragment!r}"


def test_guest_virtiofs_pip_probe_is_hermetic() -> None:
    source = (PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_virtiofs.py").read_text()

    assert "pip install --quiet cowsay" not in source
    assert "import cowsay" not in source
    assert "pip install --no-index" in source
    assert "ZipFile" in source


def test_automatic_docker_gc_never_prunes_tagged_images() -> None:
    """Concurrent worktrees must not delete each other's newly tagged images."""
    candidates = [PROJECT_ROOT / "justfile", *sorted((PROJECT_ROOT / "scripts").glob("*.sh"))]
    violations: list[str] = []
    unsafe = re.compile(r"docker\s+image\s+prune\s+[^\n]*(?:-[a-z]*a[a-z]*|--all)(?:\s|$)")
    for path in candidates:
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            if unsafe.search(line):
                violations.append(f"{path.relative_to(PROJECT_ROOT)}:{line_number}: {line.strip()}")

    assert not violations, "automatic Docker cleanup may not prune tagged images:\n" + "\n".join(
        violations
    )


def test_parallel_asset_primitive_does_not_run_docker_gc() -> None:
    """The two test-assets lanes must not run destructive cleanup against each other."""
    # The lanes call the builder directly now, and the builder does not
    # reclaim: two concurrent architectures running destructive cleanup would
    # each free what the other was still using.
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)
    argv = " ".join(build_argv(config, profile="code", arch="arm64", template="all"))
    assert "docker-gc" not in argv
    assert "gc" not in argv.split()

    lanes = _source_text("build_system/builder/gate/assetlanes.py")
    assert "gc(" not in lanes, "an asset lane reclaims while its sibling runs"
    # `_docker-gc` remains as a developer convenience; what matters is that
    # no asset lane reaches it.
    assert "_docker-gc" not in lanes


def test_release_recipes_forward_the_explicit_source_commit_to_the_gate() -> None:
    """Just dispatches; the gate owns evidence and remote-main validation."""
    justfile = _source_text("justfile")
    for recipe in ("release-binaries", "release-profile"):
        declaration = next(line for line in justfile.splitlines() if line.startswith(f"{recipe} "))
        body = _recipe_body(recipe)

        assert "source_commit" in declaration
        assert f"capsem-gate {recipe}" in body
        assert "{{quote(source_commit)}}" in body
        assert "publish-release-source.py" not in body


def test_independent_ci_owners_keep_the_fast_gate_and_fail_closed() -> None:
    scope_job = _workflow_job_block("scope")
    fast_gate = _workflow_job("fast-gate")
    gate = workflow_reachable_text(
        PROJECT_ROOT, PROJECT_ROOT / ".github" / "workflows" / "ci.yaml", job="pr-gate"
    )

    assert "fetch-depth: 0" in scope_job
    assert 'git diff --name-only -z "$base_sha"...HEAD' in scope_job
    assert scope_job.count("build_system/scripts/ci/classify-ci-scope.py --owners") == 2
    assert "owners: ${{ steps.scope.outputs.owners }}" in scope_job
    assert ".github/workflows/ci.yaml" in scope_job

    assert "if" not in fast_gate
    assert "needs" not in fast_gate

    for job_name in (
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
    ):
        job = _workflow_job_block(job_name)
        assert "needs: scope" in job
        assert (
            f"contains(fromJSON(needs.scope.outputs.owners), '{job_name}')" in job
        )

    assert "CI_OWNERS: ${{ needs.scope.outputs.owners }}" in gate
    assert "SCOPE_RESULT: ${{ needs.scope.result }}" in gate
    assert 'test "$SCOPE_RESULT" = success' in gate
    assert 'test "$FAST_GATE_RESULT" = success' in gate
    for result in (
        "TEST_LINUX_RESULT",
        "TEST_MACOS_RESULT",
        "TEST_INSTALL_RESULT",
        "DOCS_BUILD_RESULT",
        "SITE_BUILD_RESULT",
        "RELEASE_SITE_BUILD_RESULT",
    ):
        assert f'test "${result}" = skipped' in gate
        assert f'test "${result}" = success' in gate
