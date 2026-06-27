"""Ironbank DNS protocol ledger contract tests."""

from __future__ import annotations

from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import textwrap
import time
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, vm_session_db_path, vm_session_dir, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

EXPECTED_DNS_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "qname",
    "qtype",
    "qclass",
    "rcode",
    "answer_ip",
    "decision",
    "matched_rule",
    "source_proto",
    "process_name",
    "upstream_resolver_ms",
    "trace_id",
    "policy_mode",
    "policy_action",
    "policy_rule",
    "policy_reason",
    "credential_ref",
    "turn_id",
}

EXPECTED_SECURITY_COLUMNS = {
    "id",
    "timestamp_unix_ms",
    "event_id",
    "event_type",
    "rule_id",
    "rule_action",
    "detection_level",
    "rule_json",
    "event_json",
    "trace_id",
    "turn_id",
    "credential_ref",
}


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = vm_session_db_path(service.tmp_dir, service.client(), session_id)
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _query_rows(client, session_id: str, sql: str) -> list[dict]:
    db_path = vm_session_db_path(Path(client.socket_path).parent, client, session_id)
    with closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        return [dict(row) for row in conn.execute(sql).fetchall()]


def _event_id(value: object) -> str:
    assert isinstance(value, str)
    assert len(value) == 12
    assert all(ch in "0123456789abcdef" for ch in value)
    return value


def _eventually(fetch, predicate, *, timeout_s: float = 20.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = fetch()
        if predicate(last):
            return last
        time.sleep(interval_s)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


def _one_json_line(stdout: str, prefix: str) -> dict:
    line = next((line for line in stdout.splitlines() if line.startswith(prefix)), None)
    assert line is not None, stdout
    return json.loads(line.split("=", 1)[1])


def _records(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def _prove_gateway_proxy(gateway_client: TcpHttpClient) -> None:
    vm_list = gateway_client.get("/vms/list", timeout=30)
    assert isinstance(vm_list, dict)
    assert "sandboxes" in vm_list


def test_dns_query_and_block_matrix_pays_full_ledger_debt_blackbox() -> None:
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    mock_proc = None
    client = None
    session_id = vm_name("ironbank-dns")
    vm_id: str | None = None
    nonce = uuid.uuid4().hex[:12]
    allowed_qname = "fixture.capsem.test"
    blocked_qname = f"{nonce}.attacker.test"
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        request_log = service.tmp_dir / "upstream-dns-transcript.jsonl"
        mock_proc, ready = start_mock_server(request_log=request_log)
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                f"""
                refresh_policy = "24h"

                [network.dns]
                upstreams = [{json.dumps(ready["dns_udp_addr"])}]

                [corp.rules.block_ironbank_dns_exfil]
                name = "block_ironbank_dns_exfil"
                action = "block"
                priority = -100
                detection_level = "high"
                reason = "Block DNS exfiltration-shaped queries in Ironbank."
                match = 'dns.qname.matches("(^|.*\\.)attacker\\.test$")'

                [corp.rules.allow_ironbank_dns_fixture]
                name = "allow_ironbank_dns_fixture"
                action = "allow"
                priority = -90
                detection_level = "informational"
                reason = "Allow the hermetic Ironbank DNS fixture."
                match = 'dns.qname == "fixture.capsem.test"'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)
        _prove_gateway_proxy(gateway_client)

        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": {},
            },
            timeout=90,
        )
        assert create is not None
        vm_id = create["id"]
        assert create.get("name") == session_id
        assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)

        script = textwrap.dedent(
            f"""
            import json
            import socket
            import struct

            def nameserver():
                try:
                    with open("/etc/resolv.conf", encoding="utf-8") as fh:
                        for line in fh:
                            parts = line.strip().split()
                            if len(parts) == 2 and parts[0] == "nameserver":
                                return parts[1]
                except OSError:
                    pass
                return "127.0.0.1"

            def query_packet(name, query_id, qtype=1):
                labels = b"".join(bytes([len(part)]) + part.encode("ascii") for part in name.split("."))
                question = labels + b"\\0" + struct.pack("!HH", qtype, 1)
                return struct.pack("!HHHHHH", query_id, 0x0100, 1, 0, 0, 0) + question

            def advance_dns_name(message, offset):
                while True:
                    length = message[offset]
                    offset += 1
                    if length == 0:
                        return offset
                    if length & 0xC0:
                        return offset + 1
                    offset += length

            def parse_response(name, query_id, response):
                rid, flags, qdcount, ancount, _nscount, _arcount = struct.unpack("!HHHHHH", response[:12])
                assert rid == query_id, (name, rid, query_id)
                offset = 12
                for _ in range(qdcount):
                    offset = advance_dns_name(response, offset) + 4
                answer_ip = None
                answers = []
                for _ in range(ancount):
                    offset = advance_dns_name(response, offset)
                    rr_type, rr_class, ttl, rdlength = struct.unpack("!HHIH", response[offset:offset + 10])
                    offset += 10
                    rdata = response[offset:offset + rdlength]
                    offset += rdlength
                    if rr_type == 1 and rr_class == 1 and rdlength == 4:
                        answer_ip = ".".join(str(part) for part in rdata)
                    answers.append({{"type": rr_type, "class": rr_class, "ttl": ttl, "rdlength": rdlength}})
                return {{
                    "qname": name,
                    "qtype": 1,
                    "qclass": 1,
                    "query_id": query_id,
                    "rcode": flags & 0x000F,
                    "answer_count": ancount,
                    "answer_ip": answer_ip,
                    "answers": answers,
                    "response_bytes": len(response),
                }}

            def resolve(name, query_id):
                server = nameserver()
                packet = query_packet(name, query_id)
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                    sock.settimeout(10)
                    sock.sendto(packet, (server, 53))
                    response, _addr = sock.recvfrom(4096)
                result = parse_response(name, query_id, response)
                result["nameserver"] = server
                result["request_bytes"] = len(packet)
                return result

            result = {{
                "allowed": resolve({json.dumps(allowed_qname)}, 0x1201),
                "blocked": resolve({json.dumps(blocked_qname)}, 0x1202),
            }}
            print("IRONBANK_DNS_RESULT=" + json.dumps(result, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{vm_id}/files/content?path=ironbank-dns.py",
            script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True

        exec_resp = client.post(
            f"/vms/{vm_id}/exec",
            {"command": "python3 /root/ironbank-dns.py", "timeout_secs": 120},
            timeout=150,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _one_json_line(exec_resp.get("stdout") or "", "IRONBANK_DNS_RESULT=")
        assert result["allowed"]["qname"] == allowed_qname
        assert result["allowed"]["qtype"] == 1
        assert result["allowed"]["qclass"] == 1
        assert result["allowed"]["rcode"] == 0
        assert result["allowed"]["answer_count"] == 1
        assert result["allowed"]["answer_ip"] == "127.0.0.1"
        assert result["blocked"]["qname"] == blocked_qname
        assert result["blocked"]["qtype"] == 1
        assert result["blocked"]["qclass"] == 1
        assert result["blocked"]["rcode"] == 3
        assert result["blocked"]["answer_count"] == 0
        assert result["blocked"]["answer_ip"] is None

        upstream_dns = [row for row in _records(request_log) if row.get("kind") == "dns"]
        assert [
            row
            for row in upstream_dns
            if row["qname"] == allowed_qname and row["qtype"] == 1 and row["qclass"] == 1
        ], upstream_dns
        assert not [row for row in upstream_dns if row["qname"] == blocked_qname], upstream_dns

        with closing(_connect_session_db(service, vm_id)) as conn:
            assert _table_columns(conn, "dns_events") == EXPECTED_DNS_COLUMNS
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS
            dns_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM dns_events
                    WHERE qname IN (?, ?)
                    ORDER BY id
                    """,
                    (allowed_qname, blocked_qname),
                ).fetchall(),
                lambda rows: len(rows) == 2,
            )
            dns_by_name = {row["qname"]: dict(row) for row in dns_rows}
            allowed = dns_by_name[allowed_qname]
            blocked = dns_by_name[blocked_qname]

            allowed_event_id = _event_id(allowed["event_id"])
            blocked_event_id = _event_id(blocked["event_id"])
            assert allowed_event_id != blocked_event_id
            assert allowed["qtype"] == 1
            assert allowed["qclass"] == 1
            assert allowed["rcode"] == 0
            assert allowed["answer_ip"] == "127.0.0.1"
            assert allowed["decision"] == "allowed"
            assert allowed["matched_rule"] == "corp.rules.allow_ironbank_dns_fixture"
            assert allowed["source_proto"] == "udp"
            assert isinstance(allowed["upstream_resolver_ms"], int)
            assert allowed["upstream_resolver_ms"] >= 0
            assert allowed["policy_action"] == "allow"
            assert allowed["policy_rule"] == "corp.rules.allow_ironbank_dns_fixture"
            assert allowed["policy_reason"] == "Allow the hermetic Ironbank DNS fixture."
            assert isinstance(allowed["trace_id"], str) and allowed["trace_id"]
            assert allowed["credential_ref"] is None

            assert blocked["qtype"] == 1
            assert blocked["qclass"] == 1
            assert blocked["rcode"] == 3
            assert blocked["answer_ip"] is None
            assert blocked["decision"] == "denied"
            assert blocked["matched_rule"] == "corp.rules.block_ironbank_dns_exfil"
            assert blocked["source_proto"] == "udp"
            assert blocked["upstream_resolver_ms"] == 0
            assert blocked["policy_action"] == "block"
            assert blocked["policy_rule"] == "corp.rules.block_ironbank_dns_exfil"
            assert blocked["policy_reason"] == "Block DNS exfiltration-shaped queries in Ironbank."
            assert isinstance(blocked["trace_id"], str) and blocked["trace_id"]
            assert blocked["credential_ref"] is None

            security_rows = conn.execute(
                """
                SELECT *
                FROM security_rule_events
                WHERE event_id IN (?, ?)
                ORDER BY id
                """,
                (allowed_event_id, blocked_event_id),
            ).fetchall()
            assert security_rows, [allowed, blocked]
            by_rule = {(row["event_id"], row["rule_id"]): dict(row) for row in security_rows}
            allowed_security = by_rule[(allowed_event_id, "corp.rules.allow_ironbank_dns_fixture")]
            blocked_security = by_rule[(blocked_event_id, "corp.rules.block_ironbank_dns_exfil")]
            assert allowed_security["event_type"] == "dns.query"
            assert allowed_security["rule_action"] == "allow"
            assert allowed_security["detection_level"] == "informational"
            assert allowed_security["trace_id"] == allowed["trace_id"]
            allowed_event_json = json.loads(allowed_security["event_json"])
            assert allowed_event_json["event_type"] == "dns.query"
            assert allowed_event_json["dns"]["qname"] == allowed_qname
            assert allowed_event_json["dns"]["qtype"] == "1"
            assert blocked_security["event_type"] == "dns.query"
            assert blocked_security["rule_action"] == "block"
            assert blocked_security["detection_level"] == "high"
            assert blocked_security["trace_id"] == blocked["trace_id"]
            blocked_event_json = json.loads(blocked_security["event_json"])
            assert blocked_event_json["event_type"] == "dns.query"
            assert blocked_event_json["dns"]["qname"] == blocked_qname
            assert blocked_event_json["dns"]["qtype"] == "1"

        uds_rows = _query_rows(
            client,
            session_id,
            """
            SELECT event_id, qname, qtype, qclass, rcode, answer_ip, decision,
                   matched_rule, source_proto, upstream_resolver_ms, policy_action,
                   policy_rule, policy_reason, trace_id
            FROM dns_events
            WHERE qname IN ('%s', '%s')
            ORDER BY qname
            """
            % (allowed_qname, blocked_qname),
        )
        assert len(uds_rows) == 2
        assert {row["qname"] for row in uds_rows} == {allowed_qname, blocked_qname}
        assert next(row for row in uds_rows if row["qname"] == allowed_qname)["event_id"] == allowed_event_id
        assert next(row for row in uds_rows if row["qname"] == blocked_qname)["event_id"] == blocked_event_id

        security_latest = _eventually(
            lambda: client.get(f"/vms/{session_id}/security/latest?limit=100", timeout=30),
            lambda rows: {
                (row["event_id"], row["rule_id"])
                for row in rows
            }
            >= {
                (allowed_event_id, "corp.rules.allow_ironbank_dns_fixture"),
                (blocked_event_id, "corp.rules.block_ironbank_dns_exfil"),
            },
        )
        latest_by_rule = {(row["event_id"], row["rule_id"]): row for row in security_latest}
        assert latest_by_rule[(allowed_event_id, "corp.rules.allow_ironbank_dns_fixture")][
            "detection_level"
        ] == "informational"
        assert latest_by_rule[(blocked_event_id, "corp.rules.block_ironbank_dns_exfil")][
            "detection_level"
        ] == "high"

        security_status = _eventually(
            lambda: client.get(f"/vms/{session_id}/security/status", timeout=30),
            lambda payload: {
                row["detection_level"]: row["count"]
                for row in payload["by_level"]
            }.get("high", 0)
            >= 1,
        )
        by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
        by_event_type = {
            row["event_type"]: row["count"] for row in security_status["by_event_type"]
        }
        by_level = {row["detection_level"]: row["count"] for row in security_status["by_level"]}
        assert by_action["allow"] >= 1
        assert by_action["block"] >= 1
        assert by_event_type["dns.query"] >= 2
        assert by_level["informational"] >= 1
        assert by_level["high"] >= 1

        service_log = (service.tmp_dir / "service.log").read_text(encoding="utf-8")
        process_log = (
            vm_session_dir(service.tmp_dir, client, vm_id) / "process.log"
        ).read_text(encoding="utf-8")
        gateway_log = gateway.stop_and_read_log()
        assert "handle_exec" in service_log or "exec" in service_log
        assert "dns" in process_log.lower()
        assert "gateway.proxy.ok" in gateway_log
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{vm_id or session_id}/delete", timeout=60)
            except Exception:
                pass
        if gateway is not None:
            gateway.stop()
        service.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config
