"""Exercise the actual signing commands without installing or signing binaries."""

import json
import os
import subprocess
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[3]


def test_router_is_signed_without_virtualization_authority(tmp_path):
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
        ["bash", "-c", step["run"]], cwd=tmp_path, env=environment, check=True, timeout=10
    )
    signed = [json.loads(line) for line in calls.read_text().splitlines()]
    router = [args for args in signed if args[-1].endswith("/capsem-router")]
    assert len(router) == 1 and "--entitlements" not in router[0]
    assert any(args[-1].endswith("/capsem-process") and "--entitlements" in args for args in signed)

    calls.unlink()
    installed = tmp_path / "installed"
    (installed / "bin").mkdir(parents=True)
    for name in ("capsem-process", "capsem-router"):
        (installed / "bin" / name).touch()
    entitlements = ROOT / "build_system/packaging/macos/entitlements.plist"
    (tmp_path / entitlements.name).write_bytes(entitlements.read_bytes())
    source = (ROOT / "build_system/packaging/macos/pkg-scripts/postinstall").read_text()
    signing = source[
        source.index("codesign_identifier_for_bin() {") : source.index(
            'CAPSEM_INSTALL_PHASE="install_manifest_provenance"'
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
