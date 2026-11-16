"""Operator-owned delegated issuance: native gateway, stable lease and history."""

from __future__ import annotations

import json
import secrets
import sqlite3
import time
import uuid
from datetime import UTC, datetime

import msgpack
import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance
from helpers.service_gateway import ServiceGateway
from helpers.uds_client import UdsHttpClient

pytestmark = pytest.mark.integration


def test_operator_agent_access_issuance_retry_revocation_and_history():
    service = ServiceInstance(isolated_corp_config=True)
    gateway = ServiceGateway(service)
    access_id = str(uuid.uuid4())
    capability = secrets.token_hex(32)
    scope = {"operations": ["discover"], "environment_ids": [], "sessions": []}
    request = {
        "request_id": access_id,
        "ownership_token": capability,
        "scope": scope,
        "lease_seconds": 60,
    }
    try:
        service.start()
        gateway.start()
        client = TcpHttpClient(gateway.base_url, gateway.token)
        direct = UdsHttpClient(service.uds_path)
        status, _, _ = direct.call(
            "POST", "/agent-access", body=json.dumps(request).encode(),
            headers={"Authorization": f"Bearer {gateway.token}", "Content-Type": "application/json"},
        )
        assert status == 403, "operator credentials alone must not forge private transport"
        status, issued = client.call_json("POST", "/agent-access", request)
        assert status == 200, issued
        assert set(issued) == {"access"}
        assert set(issued["access"]) == {"id", "scope", "expires_at"}
        assert issued["access"]["id"] == access_id
        assert issued["access"]["scope"] == scope
        expiry = datetime.fromisoformat(issued["access"]["expires_at"].replace("Z", "+00:00"))
        assert 0 < (expiry - datetime.now(UTC)).total_seconds() <= request["lease_seconds"]
        assert capability not in json.dumps(issued)
        status, retried = client.call_json("POST", "/agent-access", request)
        assert status == 200
        assert retried == issued, "retry must preserve the original expiry"
        changed = {**request, "lease_seconds": 120}
        status, _ = client.call_json("POST", "/agent-access", changed)
        assert status == 409
        status, revoked = client.call_json("DELETE", f"/agent-access/{access_id}")
        assert status == 200
        assert revoked == {"success": True}
        status, _ = client.call_json("POST", "/agent-access", request)
        assert status == 409, "closed identifiers cannot mint replacement authority"
        service_log = service.stop_and_read_log()
        gateway_log = gateway.stop_and_read_log()
        assert capability not in service_log
        assert capability not in gateway_log
        path = service.home_dir / "sessions" / "host.db"
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as ledger:
            columns = tuple(row[1] for row in ledger.execute("PRAGMA table_info(host_events)"))
            assert columns == ("id", "timestamp_unix_ms", "kind", "session_id", "actor", "detail", "trace_id")
            rows = ledger.execute(
                "SELECT kind, actor, session_id, detail, id, timestamp_unix_ms, trace_id FROM host_events "
                "WHERE kind IN ('grant_minted','grant_revoked') ORDER BY id"
            ).fetchall()
        assert len(rows) == 2
        assert 0 < rows[0][4] < rows[1][4]
        for row in rows:
            assert 0 < row[5] <= int(datetime.now(UTC).timestamp() * 1000)
            assert row[6] is None
        assert [row[:3] for row in rows] == [
            ("grant_minted", "operator", None), ("grant_revoked", "operator", None),
        ]
        issued_detail = msgpack.unpackb(rows[0][3], raw=False)
        assert set(issued_detail) == {"access_id", "epoch", "scope", "lease_seconds"}
        assert issued_detail["access_id"] == access_id
        assert issued_detail["scope"] == scope
        assert issued_detail["lease_seconds"] == 60
        assert str(uuid.UUID(issued_detail["epoch"])) == issued_detail["epoch"]
        assert msgpack.unpackb(rows[1][3], raw=False) == {
            "access_id": access_id, "epoch": issued_detail["epoch"],
        }
        assert all(capability.encode() not in row[3] and bytes.fromhex(capability) not in row[3] for row in rows)
    finally:
        gateway.stop()
        service.stop()


def test_service_restart_does_not_reissue_historical_agent_access():
    service = ServiceInstance(isolated_corp_config=True)
    gateway = ServiceGateway(service)
    access_id = str(uuid.uuid4())
    request = {
        "request_id": access_id, "ownership_token": secrets.token_hex(32),
        "scope": {"operations": ["discover"]}, "lease_seconds": 60,
    }
    try:
        service.start()
        gateway.start()
        client = TcpHttpClient(gateway.base_url, gateway.token)
        assert client.call_json("POST", "/agent-access", request)[0] == 200
        service.stop_and_read_log()
        gateway.stop_and_read_log()
        service.start()
        gateway.start()
        client = TcpHttpClient(gateway.base_url, gateway.token)
        old_agent = TcpHttpClient(gateway.base_url, f"{access_id}.{request['ownership_token']}")
        assert old_agent.call_json("GET", "/vms/list")[0] == 401
        status, result = client.call_json("POST", "/agent-access", request)
        assert status == 409, result
        status, result = client.call_json("DELETE", f"/agent-access/{access_id}")
        assert status == 200, result
        fresh = {**request, "request_id": str(uuid.uuid4())}
        status, result = client.call_json("POST", "/agent-access", fresh)
        assert status == 200, result
        assert result["access"]["id"] == fresh["request_id"]
    finally:
        gateway.stop()
        service.stop()


def test_delegated_credentials_cannot_mutate_operator_state_on_either_transport():
    service = ServiceInstance(isolated_corp_config=True)
    gateway = ServiceGateway(service)
    access_id = str(uuid.uuid4())
    capability = secrets.token_hex(32)
    forbidden = [
        ("PATCH", "/settings/edit"),
        ("PUT", "/corp/edit"),
        ("PATCH", "/plugins/absent/edit"),
        ("PATCH", "/mcp/default/edit"),
        ("POST", "/credentials/inject"),
        ("POST", "/vms/absent/security/asks/absent/resolve"),
        ("POST", "/mcp/servers/absent/tools/absent/call"),
        ("POST", "/vms/create"),
        ("POST", "/run"),
        ("POST", "/agent-access"),
        ("DELETE", f"/agent-access/{access_id}"),
    ]
    try:
        service.start()
        gateway.start()
        operator = TcpHttpClient(gateway.base_url, gateway.token)
        status, result = operator.call_json("POST", "/agent-access", {
            "request_id": access_id, "ownership_token": capability,
            "scope": {"operations": ["discover"]}, "lease_seconds": 60,
        })
        assert status == 200, result
        agent = TcpHttpClient(gateway.base_url, f"{access_id}.{capability}")
        direct = UdsHttpClient(service.uds_path)
        policy_paths = [service.home_dir / name for name in ("settings.toml", "corp.toml")]
        policy_before = {path: path.read_bytes() if path.exists() else None for path in policy_paths}
        for method, route in forbidden:
            body = {"actor": {"kind": "operator"}, "private_payload": capability}
            status, _, result = agent.call(method, route, body=json.dumps(body).encode(),
                headers={"Content-Type": "application/json", "x-capsem-actor": "operator"})
            assert status == 403, (method, route, status, result)
            status, _, result = direct.call(method, route, body=json.dumps(body).encode(),
                headers={"Authorization": f"Bearer {access_id}.{capability}",
                         "Content-Type": "application/json", "x-capsem-actor": "operator"}, translate=False)
            assert status == 403, (method, route, status, result)
        assert {path: path.read_bytes() if path.exists() else None for path in policy_paths} == policy_before
        assert operator.call_json("GET", "/vms/list")[0] == 200
        assert operator.call_json("DELETE", f"/agent-access/{access_id}")[0] == 200
        assert agent.call_json("GET", "/vms/list")[0] == 401
        service_log = service.stop_and_read_log()
        gateway_log = gateway.stop_and_read_log()
        assert capability not in service_log
        assert capability not in gateway_log
        path = service.home_dir / "sessions" / "host.db"
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as ledger:
            rows = ledger.execute("SELECT actor,detail FROM host_events WHERE kind='access_denied' ORDER BY id").fetchall()
            assert ledger.execute("SELECT COUNT(*) FROM policy_mutation_events").fetchone()[0] == 0
            assert ledger.execute("SELECT COUNT(*) FROM host_events WHERE kind='grant_minted'").fetchone()[0] == 1
        assert len(rows) == 2 * len(forbidden) + 1
        assert all(row[0] == f"agent:{access_id}" for row in rows)
        templates = {
            "/plugins/absent/edit": "/plugins/{plugin_id}/edit",
            "/vms/absent/security/asks/absent/resolve": "/vms/{id}/security/asks/{ask_id}/resolve",
            "/mcp/servers/absent/tools/absent/call": "/mcp/servers/{server_id}/tools/{tool_id}/call",
            f"/agent-access/{access_id}": "/agent-access/{id}",
        }
        for index, (method, route) in enumerate(forbidden):
            for offset, reason in [(0, "scope_required"), (1, "gateway_transport_required")]:
                detail = msgpack.unpackb(rows[2 * index + offset][1], raw=False)
                assert set(detail) == {"method", "route", "reason"}
                assert detail["method"] == method
                assert detail["reason"] == reason
                assert detail["route"] == templates.get(route, route)
                assert capability.encode() not in rows[2 * index + offset][1]
        assert msgpack.unpackb(rows[-1][1], raw=False) == {
            "method": None, "route": None, "reason": "credential_denied",
        }
    finally:
        gateway.stop()
        service.stop()


def test_delegated_expiry_is_enforced_by_the_live_gateway_broker():
    service = ServiceInstance(isolated_corp_config=True)
    gateway = ServiceGateway(service)
    access_id = str(uuid.uuid4())
    capability = secrets.token_hex(32)
    try:
        service.start()
        gateway.start()
        operator = TcpHttpClient(gateway.base_url, gateway.token)
        status, issued = operator.call_json("POST", "/agent-access", {
            "request_id": access_id, "ownership_token": capability,
            "scope": {"operations": ["discover"]}, "lease_seconds": 5,
        })
        assert status == 200, issued
        agent = TcpHttpClient(gateway.base_url, f"{access_id}.{capability}")
        assert agent.call_json("PATCH", "/settings/edit", {})[0] == 403
        # This is the product's finite lease, not polling for ledger visibility.
        time.sleep(5.1)
        assert agent.call_json("PATCH", "/settings/edit", {})[0] == 401
        assert operator.call_json("GET", "/vms/list")[0] == 200
        service.stop_and_read_log()
        gateway.stop_and_read_log()
        path = service.home_dir / "sessions" / "host.db"
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as ledger:
            rows = ledger.execute("SELECT actor,detail FROM host_events WHERE kind='access_denied' ORDER BY id").fetchall()
        assert len(rows) == 2
        assert all(row[0] == f"agent:{access_id}" for row in rows)
        assert msgpack.unpackb(rows[-1][1], raw=False) == {
            "method": None, "route": None, "reason": "credential_denied",
        }
    finally:
        gateway.stop()
        service.stop()
