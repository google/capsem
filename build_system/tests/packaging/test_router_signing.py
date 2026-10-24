"""Exercise the actual signing commands without installing or signing binaries."""

import json
import os
import subprocess
import sys
from pathlib import Path

import yaml
from capsem_builder.gate.config import load

ROOT = Path(__file__).resolve().parents[3]


def test_confined_workers_are_signed_without_virtualization_authority(tmp_path):
    tools = tmp_path / "tools"
    tools.mkdir()
    calls = tmp_path / "calls.jsonl"
    codesign = tools / "codesign"
    codesign.write_text(
        "#!/usr/bin/env python3\n"
        "import json, os, sys\n"
        "if '--verify' in sys.argv: sys.exit(1)\n"
        "with open(os.environ['SIGNING_CALLS'], 'a') as f: f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
    )
    codesign.chmod(0o755)
    environment = {
        **os.environ,
        "PATH": f"{tools}:{os.environ['PATH']}",
        "SIGNING_CALLS": str(calls),
        "APPLE_SIGNING_IDENTITY": "test",
    }
    workflow = yaml.safe_load((ROOT / ".github/workflows/release.yaml").read_text())
    step = next(
        step
        for step in workflow["jobs"]["build-app-macos"]["steps"]
        if step.get("name") == "Codesign companion binaries"
    )
    subprocess.run(
        ["bash", "-c", step["run"]], cwd=ROOT, env=environment, check=True, timeout=10
    )
    signed = [json.loads(line) for line in calls.read_text().splitlines()]
    expected = {Path(name).name for name in load(ROOT).signing.binaries}
    assert {Path(args[-1]).name for args in signed if "--entitlements" in args} == expected
    router = [args for args in signed if args[-1].endswith("/capsem-router")]
    assert len(router) == 1 and "--entitlements" not in router[0]
    proxy = [args for args in signed if args[-1].endswith("/capsem-proxy")]
    assert len(proxy) == 1 and "--entitlements" not in proxy[0]
    ledger = [args for args in signed if args[-1].endswith("/capsem-ledger")]
    assert len(ledger) == 1 and "--entitlements" not in ledger[0]
    assert any(args[-1].endswith("/capsem-process") and "--entitlements" in args for args in signed)

    calls.unlink()
    installed = tmp_path / "installed"
    (installed / "bin").mkdir(parents=True)
    for name in {Path(args[-1]).name for args in signed}:
        (installed / "bin" / name).touch()
    entitlements = ROOT / "build_system/packaging/macos/entitlements.plist"
    (tmp_path / entitlements.name).write_bytes(entitlements.read_bytes())
    policy = subprocess.run([sys.executable, str(ROOT / "build_system/scripts/release/package-signing-roles.py")],
                            check=True, capture_output=True, text=True, timeout=10)
    (tmp_path / "virtualization-roles").write_text(policy.stdout)
    source = (ROOT / "build_system/packaging/macos/pkg-scripts/postinstall").read_text()
    signing = source[
        source.index("codesign_identifier_for_bin() {") : source.index(
            "\nCAPSEM_USER", source.index("codesign_identifier_for_bin() {")
        )
    ]
    subprocess.run(
        ["bash", "-c", signing],
        cwd=tmp_path,
        env={**environment, "CAPSEM_DIR": str(installed), "PKG_SHARE": str(tmp_path)},
        check=True,
        timeout=10,
        capture_output=True,
    )
    signed = [json.loads(line) for line in calls.read_text().splitlines()]
    router = [args for args in signed if args[-1].endswith("/capsem-router")]
    assert len(router) == 1 and "--entitlements" not in router[0]
    assert "org.capsem.router" in router[0]
    proxy = [args for args in signed if args[-1].endswith("/capsem-proxy")]
    assert len(proxy) == 1 and "--entitlements" not in proxy[0]
    assert "org.capsem.proxy" in proxy[0]
    ledger = [args for args in signed if args[-1].endswith("/capsem-ledger")]
    assert len(ledger) == 1 and "--entitlements" not in ledger[0]
    assert "org.capsem.ledger" in ledger[0]

    assert {Path(args[-1]).name for args in signed if "--entitlements" in args} == expected

    calls.unlink()
    simulation = (ROOT / "build_system/scripts/test/simulate-install.sh").read_text()
    boundary = simulation.split("# Codesign real macOS", 1)[1].split("# Copy assets", 1)[0]
    boundary = boundary.split('if [[ "$(uname -s)"', 1)[1]
    boundary = 'if [[ "$(uname -s)"' + boundary
    identifiers = source[source.index("codesign_identifier_for_bin() {"):source.index("# Preserve production")]
    subprocess.run(
        ["bash", "-eu", "-c", 'uname() { echo Darwin; }; file() { echo Mach-O; }; ' + identifiers + boundary],
        env={**environment, "REPO_ROOT": str(ROOT), "INSTALL_DIR": str(installed / "bin")},
        check=True, capture_output=True, timeout=10,
    )
    signed = [json.loads(line) for line in calls.read_text().splitlines()]
    assert {Path(args[-1]).name for args in signed if "--entitlements" in args} == expected
    calls.unlink()
    (tmp_path / "virtualization-roles").unlink()
    refused = subprocess.run(
        ["bash", "-eu", "-c", signing], cwd=tmp_path,
        env={**environment, "CAPSEM_DIR": str(installed), "PKG_SHARE": str(tmp_path)},
        capture_output=True, text=True, timeout=10,
    )
    assert refused.returncode != 0 and "signing roles missing" in refused.stderr
    assert not calls.exists()
