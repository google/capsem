"""A valid ad-hoc signature is not the production identity to preserve."""

import os
import plistlib
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]


@pytest.mark.skipif(sys.platform != "darwin", reason="real Apple codesign contract")
def test_postinstall_repairs_valid_adhoc_entitlements(tmp_path: Path) -> None:
    installed = tmp_path / "installed"
    binaries = installed / "bin"
    binaries.mkdir(parents=True)
    entitlements = ROOT / "build_system/packaging/macos/entitlements.plist"
    shutil.copyfile(entitlements, tmp_path / entitlements.name)
    trusted = binaries / "capsem"
    shutil.copyfile("/usr/bin/true", trusted)
    trusted.chmod(0o755)
    trusted_bytes = trusted.read_bytes()

    def run(*args: str) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(args, check=True, capture_output=True, timeout=15)

    for name in ("capsem-process", "capsem-router"):
        binary = binaries / name
        shutil.copyfile("/usr/bin/true", binary)
        binary.chmod(0o755)
        extra = ("--entitlements", str(entitlements)) if name == "capsem-router" else ()
        run("codesign", "--force", "--sign", "-", *extra, str(binary))
        run("codesign", "--verify", "--strict", str(binary))
    source = (ROOT / "build_system/packaging/macos/pkg-scripts/postinstall").read_text()
    start = source.index("codesign_identifier_for_bin() {")
    signing = source[start : source.index("\nCAPSEM_USER", start)]
    subprocess.run(
        ["bash", "-c", signing], check=True, capture_output=True, timeout=30,
        env={**os.environ, "CAPSEM_DIR": str(installed), "PKG_SHARE": str(tmp_path)},
    )
    assert trusted.read_bytes() == trusted_bytes, "Apple-anchored signatures must remain intact"
    process = run("codesign", "-d", "--entitlements", "-", "--xml", str(binaries / "capsem-process"))
    assert plistlib.loads(process.stdout)["com.apple.security.virtualization"] is True
    router = run("codesign", "-d", "--entitlements", "-", "--xml", str(binaries / "capsem-router"))
    assert not router.stdout or "com.apple.security.virtualization" not in plistlib.loads(router.stdout)
