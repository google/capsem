"""A real supervisor restarts the gateway only after the SDK receives its reply."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from helpers.managed_service import launchd_service
from helpers.service import ServiceInstance, materialize_test_profiles
from helpers.workspace_changes import VM_ID, seed_workspace_changes

ROOT = Path(__file__).resolve().parents[2]


def workspace_digest(root: Path) -> dict[str, str]:
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for parent in (root / "guest/workspace", root / "auto_snapshots")
            for path in parent.rglob("*") if path.is_file()}


@pytest.mark.integration
@pytest.mark.skipif(sys.platform != "darwin", reason="disposable launchd supervisor proof")
@pytest.mark.parametrize("language", ["python", "typescript"])
def test_sdk_receives_managed_restart_and_reconnects_explicitly(language: str) -> None:
    service = ServiceInstance()
    service.profiles_dir = materialize_test_profiles(service.tmp_dir)
    seed_workspace_changes(service.tmp_dir, service.profiles_dir)
    with launchd_service(service) as managed:
        before = managed.ready()
        status, inventory = before.get("/vms/list")
        assert status == 200
        registry = json.loads((service.tmp_dir / "persistent_registry.json").read_text())
        workspace = workspace_digest(service.tmp_dir / "persistent" / VM_ID)
        project = ROOT / "sdk" / language
        command = ["uv", "run", "--frozen", "python", "-m", "tests.restart_acceptance"] if language == "python" else ["node", "tools/restart-acceptance.mjs"]
        result = subprocess.run(command, cwd=project, env={
            **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
            "SDK_GATEWAY_URL": f"http://127.0.0.1:{before.port}", "SDK_GATEWAY_TOKEN": before.token,
        }, capture_output=True, text=True, timeout=20, check=False)
        assert result.returncode == 0, result.stdout + result.stderr
        assert "SDK_RESTART_ACCEPTED" in result.stdout
        after = managed.ready(before)
        assert after.get("/vms/list", token=before.token)[0] == 401
        assert after.get("/vms/list") == (200, inventory)
        assert json.loads((service.tmp_dir / "persistent_registry.json").read_text()) == registry
        assert workspace_digest(service.tmp_dir / "persistent" / VM_ID) == workspace
        reconnect = subprocess.run(
            ["uv", "run", "--frozen", "python", "-m", "tests.gateway_acceptance"]
            if language == "python" else ["node", "tools/gateway-acceptance.mjs"],
            cwd=project, env={
                **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                "SDK_GATEWAY_URL": f"http://127.0.0.1:{after.port}", "SDK_GATEWAY_TOKEN": after.token,
                "SDK_VM_ID": VM_ID,
            }, capture_output=True, text=True, timeout=30, check=False,
        )
        assert reconnect.returncode == 0, reconnect.stdout + reconnect.stderr
        assert "SDK_GATEWAY_ACCEPTANCE_OK" in reconnect.stdout
        print(f"{language}: acknowledgement delivered; service and gateway PIDs changed; token rotated; stopped VM unchanged")
