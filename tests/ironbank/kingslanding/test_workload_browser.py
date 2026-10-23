"""Guest browser requests are session metadata for every attached client.

The workload uses its normal xdg-open entrypoint, with no terminal viewer or
Xpra client attached. HTTP(S) requests reach list/info/status intact; unsafe
schemes fail in the helper without changing the current request.
"""

from uuid import UUID

import pytest
from helpers.image_session import wait_running

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service, wait_for

__all__ = ["service"]
pytestmark = pytest.mark.integration


def test_workload_browser_request_reaches_shared_inventory_without_a_viewer(service, tmp_path):
    client = service.client()
    url = "https://example.invalid/login?state=browser-proof%2Bvalue"
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "workload-browser") as vm,
    ):
        vm_id = vm["id"]
        result = client.post(f"/vms/{vm_id}/exec", {
            "command": f"xdg-open '{url}'", "timeout_secs": 5,
        }, timeout=20)
        assert result["exit_code"] == 0, result

        def request():
            rows = client.get("/vms/list")["sandboxes"]
            return next(row for row in rows if row["id"] == vm_id).get("browser_request")

        wait_for(lambda: request() is not None, "browser request in session list", timeout=10)
        expected = request()
        assert expected["id"] == 1 and expected["url"] == url
        generation = expected["owner_generation"]
        assert str(UUID(generation)) == generation
        assert client.get(f"/vms/{vm_id}/info")["browser_request"] == expected
        for value in ("file:///etc/passwd", "javascript:alert(1)"):
            refused = client.post(f"/vms/{vm_id}/exec", {
                "command": f"xdg-open '{value}'", "timeout_secs": 5,
            }, timeout=20)
            assert refused["exit_code"] != 0, refused
            assert request() == expected

        # Every client observes owner acknowledgement, including a fresh connection.
        second = service.client()
        route = f"/vms/{vm_id}/browser/acknowledge"
        first_ack = {"owner_generation": generation, "request_id": 1}
        assert second.call_json("POST", route, first_ack)[0] == 200
        wait_for(lambda: request() is None, "acknowledged browser request cleared", timeout=10)
        assert service.client().get(f"/vms/{vm_id}/info").get("browser_request") is None
        assert second.call_json("POST", route, first_ack)[0] == 200

        # An already-handled retry must not erase a newer pending URL.
        assert client.post(f"/vms/{vm_id}/exec", {
            "command": "xdg-open https://example.invalid/second", "timeout_secs": 5,
        }, timeout=20)["exit_code"] == 0
        wait_for(lambda: request() is not None, "second browser request", timeout=10)
        newer = request()
        assert newer["id"] == 2
        assert second.call_json("POST", route, first_ack)[0] == 200
        assert request() == newer

        # A cold process replacement restarts IDs but changes the owner epoch.
        client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
        client.post(f"/vms/{vm_id}/resume", {}, timeout=120)
        wait_running(client, vm_id)
        assert client.post(f"/vms/{vm_id}/exec", {
            "command": "xdg-open https://example.invalid/restarted", "timeout_secs": 5,
        }, timeout=20)["exit_code"] == 0
        wait_for(lambda: request() is not None, "replacement owner browser request", timeout=10)
        replacement = request()
        assert replacement["id"] == 1
        assert replacement["owner_generation"] != generation
        assert second.call_json("POST", route, first_ack)[0] == 409
        assert request() == replacement
        current_ack = {"owner_generation": replacement["owner_generation"], "request_id": 1}
        assert service.client().call_json("POST", route, current_ack)[0] == 200
        wait_for(lambda: request() is None, "replacement request acknowledged", timeout=10)
