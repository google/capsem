"""Existing native service/gateway/mock owners for approval acceptance."""

from __future__ import annotations

import json
import os
import sqlite3
import textwrap
import tomllib
from contextlib import closing, contextmanager, suppress
from pathlib import Path

from helpers.constants import (
    ASSETS_DIR,
    DEFAULT_CPUS,
    DEFAULT_RAM_MB,
    EXEC_READY_TIMEOUT,
)
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import (
    ServiceInstance,
    vm_name,
    vm_session_db_path,
    wait_exec_ready,
)
from helpers.session_ledger import open_session_ledger, stop_session_for_ledger_read
from helpers.settings_policy import apply_settings_rule


def apply_native_user_rule(service, rule_id):
    """Use the exact user policy exercised by the real product compiler tests."""
    fixture = (
        Path(__file__).resolve().parents[1]
        / "fixtures/config/security-rule-profile/native-approval.toml"
    )
    rule = tomllib.loads(fixture.read_text())["profiles"]["rules"][rule_id]
    return apply_settings_rule(service, rule_id, **rule)


@contextmanager
def native_approval_session(policy: str = ""):
    assert ASSETS_DIR.exists() and MOCK_SERVER_BINARY.exists()
    service = ServiceInstance()
    gateway = mock = client = session_id = None
    old_corp = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        mock, ready = start_mock_server(request_log=service.tmp_dir / "approval-upstream.jsonl")
        corp = service.tmp_dir / "corp.toml"
        corp.write_text(
            textwrap.dedent(f"""
            refresh_policy = "24h"
            [network.dns]
            upstreams = [{json.dumps(ready["dns_udp_addr"])}]
            [settings."security.web.http_upstream_ports"]
            value = [80, 3713, 8080]
            modified = "2026-06-14T00:00:00Z"
            [corp.rules.allow_native_mock_http]
            name = "allow_native_mock_http"
            action = "allow"
            priority = -90
            match = 'http.host == "127.0.0.1" && tcp.port == "3713"'
        """).strip()
            + "\n"
            + textwrap.dedent(policy).strip()
            + "\n"
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp)
        service.start()
        client = service.client()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        api = TcpHttpClient(gateway.base_url, gateway.token)
        created = client.post(
            "/vms/create",
            {
                "name": vm_name("ironbank-approval"),
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": {"CAPSEM_MOCK_SERVER_BASE_URL": ready["base_url"]},
            },
            timeout=90,
        )
        session_id = created["id"]
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)
        yield service, client, api, session_id, ready
    finally:
        if client is not None and session_id is not None:
            with suppress(Exception):
                client.delete(f"/vms/{session_id}/delete", timeout=60)
        if gateway is not None:
            gateway.stop()
        service.stop()
        stop_process(mock)
        if old_corp is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp


def upstream_records(ready: dict) -> list[dict]:
    path = Path(ready["request_log"])
    return (
        [json.loads(row) for row in path.read_text().splitlines() if row.strip()]
        if path.exists()
        else []
    )


def assert_approval_audit(service, client, session_id, receipt, *, cancelled=False, table=None):
    return assert_approval_audits(service, client, session_id, [(receipt, cancelled)], table=table)


def assert_approval_audits(service, client, session_id, receipts, *, table=None):
    """Stop our named fixture to drain all operation producers before disk reads."""
    path = vm_session_db_path(service.tmp_dir, client, session_id)
    stop_session_for_ledger_read(client, session_id, path)
    with closing(open_session_ledger(path)) as conn:
        conn.row_factory = sqlite3.Row
        for receipt, cancelled in receipts:
            _assert_approval_rows(conn, receipt, cancelled=cancelled, table=table)


def _assert_approval_rows(conn, receipt, *, cancelled, table):
    rows = conn.execute(
        "SELECT status,resolver,reason,event_id FROM security_ask_events "
        "WHERE ask_id=? ORDER BY id",
        (receipt["ask_id"],),
    ).fetchall()
    expired = receipt["status"] == "expired"
    terminal = "denied" if cancelled or expired else receipt["status"]
    assert [row["status"] for row in rows] == ["pending", terminal], [dict(row) for row in rows]
    assert rows[0]["event_id"] == rows[1]["event_id"] == receipt["event_id"]
    assert rows[1]["resolver"] == "authenticated_client"
    assert rows[1]["reason"] == (
        "request_cancelled" if cancelled else "deadline" if expired else "human_decision"
    )
    if table is not None:
        assert table in {
            "dns_events",
            "tool_calls",
            "exec_events",
            "fs_events",
            "net_events",
        }
        count = conn.execute(
            f"SELECT COUNT(*) FROM {table} WHERE event_id=?", (receipt["event_id"],)
        ).fetchone()[0]
        assert count == 1, (table, receipt, count)
