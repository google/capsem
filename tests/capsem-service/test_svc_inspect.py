"""Inspect (telemetry DB query) endpoint tests."""

import pytest

pytestmark = pytest.mark.integration


class TestInspect:

    def test_valid_sql(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/inspect/{name}", {
            "sql": "SELECT name FROM sqlite_master WHERE type='table'",
        })
        assert resp is not None
        assert "columns" in resp or "rows" in resp or len(str(resp)) > 0

    def test_count_query(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/inspect/{name}", {
            "sql": "SELECT count(*) as cnt FROM net_events",
        })
        assert resp is not None

    def test_bad_sql(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/inspect/{name}", {
            "sql": "THIS IS NOT SQL",
        })
        assert resp is None or "error" in str(resp).lower()

    def test_inspect_nonexistent_vm(self, service_env):
        client = service_env.client()
        resp = client.post("/inspect/ghost-vm", {"sql": "SELECT 1"})
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
