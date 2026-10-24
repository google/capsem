"""Exercise the local candidate's real disposable macOS bundle seal."""

import json
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
    entitlements = _run(["codesign", "-d", "--entitlements", ":-", str(app)])
    assert "com.apple.security.virtualization" not in entitlements.stdout + entitlements.stderr
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
