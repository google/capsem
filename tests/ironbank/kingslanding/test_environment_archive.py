"""Archive admission through the gateway and a live persistent sandbox."""

import secrets
import uuid
from contextlib import closing

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import TcpHttpClient
from helpers.service import vm_name
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import create, request, save
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_archiving_an_environment_preserves_existing_pinned_work(service):
    specification = {"name": "archive-lifecycle", "variables": {"SAVED_VALUE": "old"}}
    original = save(service, specification)
    existing_name = vm_name("archived-existing")
    existing = create(service, original, existing_name)
    root = f"/environments/{original['environment_id']}"
    revision_path = f"{root}/revisions/{original['revision_id']}/info"

    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            def change(body):
                return operator.call_json("POST", f"{root}/changes", body)

            code, edited = change({
                "request_id": str(uuid.uuid4()),
                "base_revision_id": original["revision_id"],
                "reason": "Record a newer head while the sandbox keeps its pin",
                "specification": {**specification, "variables": {"SAVED_VALUE": "new"}},
            })
            assert code == 200 and edited["application"] == "applied", edited
            current = edited["result"]
            historical = operator.get(revision_path)
            archive = {
                "action": "archive",
                "request_id": str(uuid.uuid4()),
                "base_revision_id": original["revision_id"],
                "reason": "Retire this blueprint for future sandbox creation",
            }
            assert change(archive)[0] == 409
            archive["base_revision_id"] = current["revision_id"]
            owner = operator.get(f"/vms/{existing}/info")["environment"]
            secret = secrets.token_hex(32)
            code, grant = operator.call_json("POST", "/agent-access", {
                "request_id": str(uuid.uuid4()),
                "ownership_token": secret,
                "lease_seconds": 300,
                "scope": {"operations": ["propose"], "sessions": [{
                    "session_id": existing, "owner_generation": owner["owner_generation"],
                }]},
            })
            assert code == 200, grant
            with closing(TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")) as agent:
                assert agent.call_json("POST", f"{root}/changes", archive)[0] == 403
            code, receipt = change(archive)
            assert code == 200 and receipt["application"] == "applied", receipt
            assert change(archive) == (200, receipt)
            assert change({**archive, "reason": "Different intent"})[0] == 409
            assert receipt["result"] == current
            details = operator.get(f"/requests/{archive['request_id']}/info")
            assert details["summary"]["kind"] == "environment_archive"
            assert details["summary"]["result"] == current
            assert operator.get(f"{root}/info")["archived"] is True
            [listed] = operator.get("/environments/list?search=archive-lifecycle")["environments"]
            assert listed["archived"] is True and listed["instance_count"] == 1
            assert operator.get(revision_path) == historical

            for revision in (original, current):
                status, refused = operator.call_json("POST", "/vms/create", {
                    "name": vm_name("archived-refused"),
                    "environment": revision,
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                })
                assert status == 409, refused

            clone = {"name": vm_name("archived-clone"), "from": existing_name}
            assert operator.call_json("POST", "/vms/create", clone)[0] == 409
            fork = {"name": vm_name("archived-fork")}
            assert operator.call_json("POST", f"/vms/{existing}/fork", fork)[0] == 409

            assert operator.get(f"/vms/{existing}/info")["environment"]["boot"] == original
            assert request(service, "POST", f"/vms/{existing}/exec", {"command": "test \"$SAVED_VALUE\" = old"})[
                "exit_code"
            ] == 0
            request(service, "POST", f"/vms/{existing}/stop", {})
    finally:
        gateway.stop()

    service.stop(cleanup=False)
    service.start()
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as recovered_operator:
            assert recovered_operator.get(f"{root}/info")["archived"] is True
            assert recovered_operator.call_json("POST", f"{root}/changes", archive) == (200, receipt)
            request(service, "POST", f"/vms/{existing}/start", {})
            assert recovered_operator.get(f"/vms/{existing}/info")["environment"]["boot"] == original
            assert request(service, "POST", f"/vms/{existing}/exec", {"command": "test \"$SAVED_VALUE\" = old"})[
                "exit_code"
            ] == 0
    finally:
        gateway.stop()
