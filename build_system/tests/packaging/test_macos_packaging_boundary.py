"""Own every macOS package, signing, and package-proof resource together."""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest
from helpers.source_modes import tracked_source_modes

ROOT = Path(__file__).resolve().parents[3]
MACOS = ROOT / "build_system" / "packaging" / "macos"
LEGACY = ROOT / ("scr" + "ipts")


@pytest.mark.parametrize("status", [0, 23])
def test_physical_boot_cleanup_retains_logs_and_exit_status(tmp_path: Path, status: int) -> None:
    script = tmp_path / "build_system/packaging/macos/prove-macos-package-boot.sh"
    script.parent.mkdir(parents=True)
    # Run the real setup/EXIT trap without installing a package or starting VZ.
    setup = (MACOS / script.name).read_text().split('rm -rf "$WORK_ROOT"\n', 1)[0]
    script.write_text(setup + '''
mkdir -p "$RUN_DIR/persistent/vm" "$WORK_ROOT"
printf '%s' "$RUN_DIR" > "$WORK_ROOT/runtime-path"
printf 'checkpoint failed\\n' > "$RUN_DIR/persistent/vm/serial.log"
printf 'service evidence\\n' > "$RUN_DIR/service.2026-09-09.log"
printf 'large VM state is not diagnostic evidence' > "$RUN_DIR/persistent/vm/rootfs.img"
exit "$PROOF_EXIT_STATUS"
''')
    package = tmp_path / "fixture.pkg"
    package.write_text("fixture")
    result = subprocess.run(
        ["bash", "-c", 'uname() { echo Darwin; }; pkill() { :; }; source "$@"',
         "proof", str(script), "--package", str(package), "--version", "0.0.0",
         "--assets-dir", str(tmp_path), "--work-root", str(tmp_path / "proof")],
        env={"PATH": "/usr/bin:/bin", "PROOF_EXIT_STATUS": str(status)},
        capture_output=True, text=True, timeout=10,
    )
    assert result.returncode == status, result.stderr
    work = tmp_path / "proof"
    assert not Path((work / "runtime-path").read_text()).exists()
    logs = work / "diagnostics"
    assert (logs / "persistent/vm/serial.log").read_text() == "checkpoint failed\n"
    assert (logs / "service.2026-09-09.log").read_text() == "service evidence\n"
    assert not (logs / "persistent/vm/rootfs.img").exists()

EXPECTED_MODES = {
    "build-pkg.sh": 0o755,
    "build-test-macos-package.sh": 0o755,
    "entitlements.plist": 0o644,
    "fix_p12_legacy.sh": 0o755,
    "install-local-macos-package.applescript": 0o644,
    "macos-install-user-request.sh": 0o755,
    "macos-tart-binary-apply.sh": 0o644,
    "macos-tart-regression-probes.sh": 0o644,
    "macos_binary_apply.py": 0o644,
    "macos_candidate_content.py": 0o644,
    "macos_release_glowup.py": 0o755,
    "macos_tart_cleanup.py": 0o644,
    "macos_host_admission.py": 0o644,
    "macos-tart-host-admission.sh": 0o644,
    "macos_tart_glowup.py": 0o755,
    "macos_tart_guest.sh": 0o755,
    "macos_tart_transition_support.py": 0o644,
    "pkg-distribution.xml": 0o644,
    "pkg-scripts/install-user": 0o755,
    "pkg-scripts/postinstall": 0o755,
    "pkg-scripts/preinstall": 0o755,
    "prove-macos-package-boot.sh": 0o755,
    "run_signed.sh": 0o755,
}


def test_macos_packaging_resources_have_one_exact_owner() -> None:
    found = {
        path.relative_to(MACOS).as_posix()
        for path in MACOS.rglob("*")
        if path.is_file()
        and "__pycache__" not in path.parts
        and not path.name.startswith(".")
    }

    assert found == set(EXPECTED_MODES)
    assert not (ROOT / "entitlements.plist").exists()
    for name in EXPECTED_MODES:
        if name != "entitlements.plist":
            assert not (LEGACY / name).exists()


def test_macos_packaging_resources_preserve_reviewed_modes() -> None:
    assert tracked_source_modes(ROOT, MACOS) == EXPECTED_MODES


def test_postinstall_never_edits_shell_profiles_as_root() -> None:
    source = (MACOS / "pkg-scripts/postinstall").read_text(encoding="utf-8")
    shell = source.split('CAPSEM_INSTALL_PHASE="configure_shell"', 1)[1].split(
        '# Register LaunchAgent', 1
    )[0].strip()
    commands = "\n".join(line for line in shell.splitlines() if not line.startswith("#")).strip()
    assert commands.startswith('su "$USER" -c \'')
    assert commands.endswith("'")
    assert '>> "$PROFILE"' in shell
    assert '>> "$FISH_CONFIG"' in shell
    assert "chown" not in shell


@pytest.mark.parametrize("name", ["preinstall", "postinstall"])
def test_package_root_script_does_not_write_through_user_home(name: str) -> None:
    source = (MACOS / "pkg-scripts" / name).read_text(encoding="utf-8")
    write = re.compile(
        r'^\s*(?:mkdir|touch|cp|chmod|chown|rm|install|ln|codesign|printf)\b.*'
        r'\$(?:CAPSEM_DIR|USER_HOME|INSTALL_LOG|INSTALL_RUN_LOG|INSTALL_RUN_FILE)'
    )
    in_user_script = False
    violations = []
    for line in source.splitlines():
        if line.lstrip().startswith('su "$USER" -c ') and line.endswith("<<'CAPSEM_USER'"):
            in_user_script = True
        elif line == "CAPSEM_USER":
            in_user_script = False
        elif not in_user_script and write.search(line):
            violations.append(line.strip())
    assert not violations, f"root writes through installing user's home: {violations}"
    if name == "postinstall":
        assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG")' not in source
    else:
        assert 'INSTALL_LOG="/var/run/capsem/install.log"' in source


def test_macos_failure_report_is_written_under_installing_uid() -> None:
    source = (ROOT / "build_system/packaging/shared/install-diagnostics").read_text(encoding="utf-8")
    assert 'CAPSEM_INSTALL_FAILURE_AS_USER' in source
    assert 'su "$install_user" -c' in source
    for name in ("preinstall", "postinstall"):
        script = (MACOS / "pkg-scripts" / name).read_text(encoding="utf-8")
        assert 'CAPSEM_INSTALL_FAILURE_AS_USER=1' in script


def test_user_selected_local_manifest_is_opened_after_dropping_root() -> None:
    source = (MACOS / "pkg-scripts/postinstall").read_text(encoding="utf-8")
    helper = (ROOT / "build_system/packaging/shared/install-manifest").read_text(encoding="utf-8")
    assert 'if [[ "$MANIFEST_SOURCE" == file://* && -z "$MANIFEST_PAYLOAD" ]]' in source
    assert 'cat "${manifest_source#file://}"' in helper
    assert 'su "$USER" -c "/bin/bash -c' in source


def test_gate_selects_macos_packaging_resources_from_their_owner() -> None:
    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    prefix = "build_system/packaging/macos/"

    assert config["install"]["local_macos_package_script"] == (
        prefix + "build-test-macos-package.sh"
    )
    assert config["install"]["local_macos_installer"] == [
        "/usr/bin/osascript",
        prefix + "install-local-macos-package.applescript",
    ]
    assert config["signing"]["entitlements"] == prefix + "entitlements.plist"
    assert config["modules"]["macos_glowup_script"] == (
        prefix + "macos_release_glowup.py"
    )


def test_macos_package_assembly_uses_owner_relative_resources() -> None:
    builder = (MACOS / "build-pkg.sh").read_text(encoding="utf-8")
    candidate = (MACOS / "build-test-macos-package.sh").read_text(encoding="utf-8")
    runner = (MACOS / "run_signed.sh").read_text(encoding="utf-8")

    assert 'bash "$SCRIPT_DIR/build-pkg.sh"' in candidate
    assert '"$SCRIPT_DIR/entitlements.plist"' in builder
    assert '"$SCRIPT_DIR/pkg-scripts/$package_script"' in builder
    assert '"$SCRIPT_DIR/../shared/$package_script"' in builder
    assert 'ENTITLEMENTS="$SCRIPT_DIR/entitlements.plist"' in runner


def test_release_workflow_selects_owned_macos_resources() -> None:
    source = (ROOT / ".github" / "workflows" / "release.yaml").read_text(
        encoding="utf-8"
    )
    prefix = "build_system/packaging/macos/"

    assert f"--entitlements {prefix}entitlements.plist" in source
    assert f"bash {prefix}build-pkg.sh" in source
    assert f"bash {prefix}macos-install-user-request.sh write" in source


def test_local_package_consumes_the_binaries_cargo_produced(tmp_path: Path) -> None:
    """Exercise the assembly handoff without compiling or installing a package."""
    script = tmp_path / "build_system/packaging/macos/build-test-macos-package.sh"
    script.parent.mkdir(parents=True)
    script.write_bytes((MACOS / script.name).read_bytes())
    (tmp_path / "Cargo.toml").write_text('version = "0.0.0"\n')
    release = tmp_path / "cache/target/cargo/release"
    (release / "bundle/macos/Capsem.app").mkdir(parents=True)
    (release / "capsem").write_text("compiled payload\n")
    content = tmp_path / "content"
    content.mkdir()

    result = subprocess.run(
        [
            "bash", "-c", r'''
            # This assembly-only fixture models the explicit CI source binding.
            export CAPSEM_BUILD_REVISION=1111111111111111111111111111111111111111
            cargo() { :; }
            codesign() { :; }
            uname() { echo Darwin; }
            bash() {
                if [[ "${1##*/}" == build-pkg.sh ]]; then
                    [[ -d "$4" && -f "$5/capsem" ]] || {
                        echo "assembly cannot find Cargo's payload in $5" >&2
                        exit 1
                    }
                    echo PACKAGE_INPUTS_FOUND
                    exit 0
                fi
            }
            source "$1" --assets-dir "$2" --sbom "$2/sbom.spdx.json"
            ''',
            "package-handoff", str(script), str(content),
        ],
        capture_output=True, text=True, check=False, timeout=10,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert "PACKAGE_INPUTS_FOUND" in result.stdout


@pytest.mark.parametrize("authority", ["primary", "linked", "configured"])
def test_macos_package_outputs_share_the_admitted_cache_owner(
    tmp_path: Path, authority: str
) -> None:
    """Run both real shell resolutions against real policy/common-Git metadata."""
    primary = tmp_path / "primary"
    checkout = primary if authority == "primary" else tmp_path / "worktree"
    checkout.mkdir(parents=True)
    (checkout / "config").mkdir()
    shutil.copy2(ROOT / "config/cache.toml", checkout / "config/cache.toml")
    git_dir = primary / ".git/worktrees/fixture"
    git_dir.mkdir(parents=True)
    (git_dir / "commondir").write_text("../..\n")
    if checkout != primary:
        (checkout / ".git").write_text(f"gitdir: {git_dir}\n")
    helper = checkout / "build_system/scripts/release/package-output-directory.py"
    helper.parent.mkdir(parents=True)
    if (ROOT / "build_system/scripts/release" / helper.name).exists():
        shutil.copy2(ROOT / "build_system/scripts/release" / helper.name, helper)
    bootstrap = checkout / "build_system/builder/bootstrap.py"
    bootstrap.parent.mkdir()
    shutil.copy2(ROOT / "build_system/builder/bootstrap.py", bootstrap)
    scripts = checkout / "build_system/packaging/macos"
    scripts.mkdir(parents=True)
    env = os.environ.copy()
    env["PATH"] = str(Path(sys.executable).parent) + os.pathsep + env["PATH"]
    env.pop("CAPSEM_CACHE_AUTHORITY", None)
    expected_root = primary
    if authority == "configured":
        expected_root = tmp_path / "explicit-owner"
        env["CAPSEM_CACHE_AUTHORITY"] = str(expected_root)
    from capsem_builder.cache.config import load_paths
    expected = load_paths(checkout, environment=env).stage("packages")
    assert expected.is_relative_to(expected_root)
    for name in ["build-pkg.sh", "build-test-macos-package.sh"]:
        source = (MACOS / name).read_text()
        declarations = [line for line in source.splitlines() if line.startswith("PACKAGE_DIR=")]
        variable = "OUTPUT_PKG" if name == "build-pkg.sh" else "PKG"
        declarations.append(next(line for line in source.splitlines() if line.startswith(variable + "=")))
        result = subprocess.run(
            ["bash", "-c", "\n".join(declarations) + f'\nprintf "%s" "${variable}"', "package-output"],
            cwd=checkout, env={**env, "SCRIPT_DIR": str(scripts), "ROOT": str(checkout), "VERSION": "0.7.1"},
            capture_output=True, text=True, timeout=10, check=False,
        )
        assert result.returncode == 0, result.stderr
        assert Path(result.stdout) == expected / "Capsem-0.7.1.pkg"
    assert not expected.exists(), "resolving the admitted destination must not write"


def test_package_output_resolver_enters_the_locked_project_interpreter() -> None:
    # macOS shell python is 3.9; the existing bootstrap must enter the locked
    # builder before importing its typed cache/config dependencies.
    from capsem_builder.cache.config import load_paths
    result = subprocess.run(
        ["/usr/bin/python3", str(ROOT / "build_system/scripts/release/package-output-directory.py")],
        cwd="/", env={"PATH": "/usr/bin:/bin"},
        capture_output=True, text=True, timeout=15, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert Path(result.stdout.strip()) == load_paths(ROOT).stage("packages")
