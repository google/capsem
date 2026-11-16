"""Operator-owned delegated issuance: native gateway, stable lease and history."""

from __future__ import annotations

import json
import secrets
import sqlite3
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
