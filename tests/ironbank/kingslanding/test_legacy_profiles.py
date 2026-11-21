"""A profile-era persistent VM stays incompatible on the native service."""

import json

import psutil
import pytest
from helpers.constants import ASSETS_DIR
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance
from helpers.service_gateway import ServiceGateway
from helpers.stopped_workspace import VM_ID, VM_NAME, seed_stopped_workspace

pytestmark = pytest.mark.integration


def test_profile_era_vm_refuses_start_and_resume_without_spawning():
    service = ServiceInstance(isolated_corp_config=True)
    gateway = ServiceGateway(service)
    api = None
    try:
        seed_stopped_workspace(service.tmp_dir, ASSETS_DIR)
        registry = service.tmp_dir / "persistent_registry.json"
        before = json.loads(registry.read_text())
        entry = before["vms"][VM_NAME]
        entry.update(profile_id="code", base_version="0.6.6")
        registry.write_text(json.dumps(before))

        service.start()
        gateway.start()
        api = TcpHttpClient(gateway.base_url, gateway.token)
        diagnostic = "created from the 'code' profile, and profiles no longer exist"
        for operation in ("start", "resume"):
            rows = api.get("/vms/list")["sandboxes"]
            row = next(row for row in rows if row["id"] == VM_ID)
            assert row["status"] == "Incompatible", row
            assert row["can_resume"] is False, row
            assert diagnostic in row["resume_blocked_reason"], row

            code, body = api.call_json("POST", f"/vms/{VM_ID}/{operation}", {})
            assert code >= 400 and diagnostic in body["error"], (code, body)
            assert service.proc is not None
            children = psutil.Process(service.proc.pid).children()
            assert all(child.name() != "capsem-process" for child in children)
            on_disk = json.loads(registry.read_text())["vms"][VM_NAME]
            assert on_disk["profile_id"] == "code"
            assert on_disk["session_dir"] == entry["session_dir"]
    finally:
        if api is not None:
            api.close()
        gateway.stop()
        service.stop()
