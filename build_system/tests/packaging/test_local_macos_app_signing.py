"""Exercise the local candidate's real disposable macOS bundle seal."""

import json
import os
import plistlib
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
CALLER = ROOT / "build_system/packaging/macos/build-test-macos-package.sh"


def _run(argv: list[str], **kwargs) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, capture_output=True, text=True, timeout=15, check=False, **kwargs)


def _bundle(tmp_path: Path) -> Path:
    app = tmp_path / "cache/target/cargo/release/bundle/macos/Capsem.app"
    executable = app / "Contents/MacOS/capsem-app"
    executable.parent.mkdir(parents=True)
    shutil.copyfile("/usr/bin/true", executable)
    executable.chmod(0o755)
    # A standalone ad-hoc Mach-O copied into a bundle has no resource seal,
    # exactly the original Tauri linker-signature boundary (not a candidate).
    result = _run(["codesign", "--sign", "-", "--force", str(executable)])
    assert result.returncode == 0, result.stderr
    (app / "Contents/Resources").mkdir()
    (app / "Contents/Resources/owned.txt").write_text("fresh local fixture")
    (app / "Contents/Info.plist").write_bytes(plistlib.dumps({
        "CFBundleExecutable": "capsem-app", "CFBundleIdentifier": "com.capsem.capsem",
        "CFBundlePackageType": "APPL", "CFBundleVersion": "1",
    }))
    return app


@pytest.mark.skipif(shutil.which("codesign") is None, reason="macOS codesign required")
def test_fresh_local_candidate_seals_bundle_without_vm_authority(tmp_path: Path) -> None:
    app = _bundle(tmp_path)
    before = _run(["codesign", "--verify", "--strict", str(app)])
    assert before.returncode != 0
    source = CALLER.read_text()
    # Execute the original post-Tauri/pre-companion boundary, never Cargo or pkgbuild.
    boundary = source.split('cargo tauri build --bundles app', 1)[1].split("\n", 1)[1]
    boundary = boundary.split("cargo build --release", 1)[0]
    result = _run(["bash", "-eu", "-c", boundary], env={"ROOT": str(tmp_path), "PATH": "/usr/bin:/bin"})
    assert result.returncode == 0, result.stderr
    verified = _run(["codesign", "--verify", "--strict", str(app)])
    assert verified.returncode == 0, verified.stderr
    assert (app / "Contents/_CodeSignature/CodeResources").is_file()
    entitlements = _run(["codesign", "-d", "--entitlements", "-", "--xml", str(app)])
    actual = plistlib.loads(entitlements.stdout.encode()) if entitlements.stdout else {}
    assert "com.apple.security.virtualization" not in actual
    assert "--deep" not in boundary and "--entitlements" not in boundary
    # The owned resource seal must actually detect a post-sign mutation.
    (app / "Contents/Resources/owned.txt").write_text("tampered")
    assert _run(["codesign", "--verify", "--strict", str(app)]).returncode != 0


def test_production_bundle_signing_rail_is_not_replaced() -> None:
    config = json.loads((ROOT / "crates/capsem-app/tauri.conf.json").read_text())
    assert config["identifier"] == "com.capsem.capsem"
    workflow = (ROOT / ".github/workflows/release.yaml").read_text()
    assert "build-test-macos-package.sh" not in workflow
    assert "APPLE_SIGNING_IDENTITY" in workflow


def test_valid_developer_id_bundle_is_verified_without_resigning() -> None:
    source = CALLER.read_text()
    boundary = source.split('cargo tauri build --bundles app', 1)[1].split("\n", 1)[1]
    boundary = boundary.split("cargo build --release", 1)[0]
    result = _run(["bash", "-eu", "-c", 'codesign() { printf "%s\\n" "$*"; }; ' + boundary],
                  env={"ROOT": "/fresh-owned-output", "PATH": "/usr/bin:/bin"})
    assert result.returncode == 0, result.stderr
    assert "--test-requirement =anchor apple generic" in result.stdout
    assert "--sign" not in result.stdout
    assert result.stdout.count("--verify --strict") == 2


def _installed_cohort_probe(tmp_path: Path, signature: str, strict_status: int = 0,
                            damage: str = "", conditional: bool = False,
                            expected_version: str = "0.6.6", reported_version: str | None = None) -> tuple[subprocess.CompletedProcess[str], str]:
    """Actual sourced probe, with controlled external codesign outcomes; no signing."""
    binary_dir = tmp_path / "bin"
    binary_dir.mkdir()
    for role in ("capsem", "capsem-service"):
        if damage == "missing_role" and role == "capsem":
            continue
        version = reported_version if reported_version is not None else (
            "wrong-version" if damage == "version" else expected_version
        )
        path = binary_dir / role
        path.write_text(f'#!/bin/bash\nprintf "%s\\n" "{role} {version}"\n')
        path.chmod(0o755)
    if damage == "missing_verifier":
        grep = shutil.which("grep")
        assert grep is not None
        (binary_dir / "grep").symlink_to(grep)
    metadata = ("Signature=adhoc" if signature == "adhoc" else
                "Authority=Developer ID Application: Elie Bursztein (L8EGK4X86T)\n"
                "Authority=Developer ID Certification Authority\n"
                "Authority=Apple Root CA\nTeamIdentifier=L8EGK4X86T")
    # Reduced public066 display grammar from preserved S113 diagnosis. It is
    # diagnostic text only; strict verification, never a text match, decides trust.
    (tmp_path / "display").write_text(metadata + "\n")
    script = r'''
set -euo pipefail
SHARE="$1"
source "$2"
CAPSEM_BIN_DIR="$SHARE/bin"
VERSION="$EXPECTED_VERSION"
BINARIES=(capsem capsem-service)
codesign() {
    printf '%s\n' "$*" >> "$SHARE/codesign-calls"
    case "$1" in
        --verify) test "$2" = --strict; return "$STRICT_STATUS" ;;
        -d) cat "$SHARE/display" ;;
        *) return 97 ;;
    esac
}
if test "$MISSING_VERIFIER" = 1; then unset -f codesign; PATH="$SHARE/bin"; fi
if test "$CONDITIONAL" = 1; then
    if verify_binary_cohort; then exit 0; else exit "$?"; fi
else
    verify_binary_cohort
fi
'''
    result = _run(["bash", "-c", script, "probe", str(tmp_path),
                   str(ROOT / "build_system/packaging/macos/macos-tart-regression-probes.sh")],
                  env={**os.environ, "STRICT_STATUS": str(strict_status),
                       "CONDITIONAL": "1" if conditional else "0",
                       "MISSING_VERIFIER": "1" if damage == "missing_verifier" else "0",
                       "EXPECTED_VERSION": expected_version})
    calls = (tmp_path / "codesign-calls").read_text() if (tmp_path / "codesign-calls").exists() else ""
    return result, calls


@pytest.mark.parametrize("signature", ["developer_id", "adhoc"])
def test_installed_cohort_preserves_valid_signatures(tmp_path: Path, signature: str) -> None:
    result, calls = _installed_cohort_probe(tmp_path, signature)
    assert result.returncode == 0, result.stderr
    assert calls.count("--verify --strict") == 2
    assert "--sign" not in calls
    assert "Authority=" not in result.stdout + result.stderr


@pytest.mark.parametrize("signature", ["developer_id", "adhoc"])
@pytest.mark.parametrize("status", [1, 42, 43], ids=["missing", "corrupt", "verifier_failure"])
def test_installed_cohort_propagates_strict_failure_even_in_conditional(
    tmp_path: Path, signature: str, status: int,
) -> None:
    result, calls = _installed_cohort_probe(tmp_path, signature, status, conditional=True)
    assert result.returncode == status
    assert calls.count("--verify --strict") == 1
    assert "capsem-service" not in calls
    assert "role=capsem stage=strict-signature failed" in result.stderr


@pytest.mark.parametrize("damage", ["missing_role", "version"])
def test_installed_cohort_keeps_role_and_version_refusals(tmp_path: Path, damage: str) -> None:
    result, calls = _installed_cohort_probe(tmp_path, "adhoc", damage=damage, conditional=True)
    assert result.returncode != 0
    assert not calls
    stage = "executable" if damage == "missing_role" else "version"
    assert f"role=capsem stage={stage} failed" in result.stderr


def test_installed_cohort_requires_codesign_tool(tmp_path: Path) -> None:
    result, calls = _installed_cohort_probe(tmp_path, "adhoc", damage="missing_verifier", conditional=True)
    assert result.returncode == 127
    assert not calls
    assert "role=capsem stage=strict-signature failed" in result.stderr


@pytest.mark.parametrize("reported", ["0.7.10", "0.7.1-foreign"])
def test_installed_cohort_rejects_version_prefixes(tmp_path: Path, reported: str) -> None:
    result, calls = _installed_cohort_probe(
        tmp_path, "adhoc", expected_version="0.7.1", reported_version=reported, conditional=True,
    )
    assert result.returncode != 0
    assert not calls
    assert "role=capsem stage=version failed" in result.stderr


def test_installed_cohort_accepts_exact_version_with_build_annotation(tmp_path: Path) -> None:
    result, calls = _installed_cohort_probe(
        tmp_path, "developer_id", expected_version="0.7.1",
        reported_version="0.7.1 (build abc123 ts=20261024)",
    )
    assert result.returncode == 0, result.stderr
    assert calls.count("--verify --strict") == 2
