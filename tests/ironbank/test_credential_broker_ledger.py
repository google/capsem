"""Ironbank credential broker ledger contract tests."""

from __future__ import annotations

import json
import shlex
import stat
from contextlib import closing

import psutil
import pytest
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.image_session import image_session, workload_exec
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import ServiceInstance, vm_name, vm_session_dir
from log_streams import read_log_stream

from tests.ironbank.test_credential_store_lifecycle import _credential_reference
from tests.ironbank.test_http_protocol_ledger import (
    EXPECTED_NET_COLUMNS,
    EXPECTED_SECURITY_COLUMNS,
    EXPECTED_SUBSTITUTION_COLUMNS,
    _connect_session_db,
    _eventually,
    _table_columns,
)
from tests.ironbank.test_http_protocol_ledger import (
    test_brokered_http_rewrite_pays_full_ledger_debt_blackbox as _broker_rewrite_proof,
)

pytestmark = pytest.mark.integration


def test_credential_broker_capture_injects_and_reports_full_ledger_blackbox() -> None:
    """Dedicated S01-005 entry point for the broker rewrite ledger proof."""
    _broker_rewrite_proof()


@pytest.mark.parametrize("source", ("api", "startup"))
def test_explicit_file_and_memory_injection_reaches_a_running_owner(monkeypatch, source: str) -> None:
    """Host secrets reach upstream; guest, persisted memory and audit stay clean."""
    service = ServiceInstance()
    gateway = GatewayInstance(uds_path=service.uds_path)
    upstream = None
    secrets = {"memory": "capsem_test_host_memory_only", "file": "capsem_test_host_file"}
    unselected = "capsem_test_host_not_selected"
    markers = (*secrets.values(), unselected)
    try:
        upstream, ready = start_mock_server(request_log=service.tmp_dir / "upstream.jsonl")
        corp = service.tmp_dir / "corp.toml"
        corp.write_text(f'''
refresh_policy = "24h"
[network.dns]
upstreams = [{json.dumps(ready["dns_udp_addr"])}]
[network.upstream_overrides."egress.capsem.test:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"
[settings."vm.resources.log_bodies"]
value = true
modified = "2026-06-14T00:00:00Z"
[settings."security.web.http_upstream_ports"]
value = [80, 3713, 8080]
modified = "2026-06-14T00:00:00Z"
[ai.google]
allowed_remote_targets = ["egress.capsem.test:443"]
[ai.google.rules.ironbank_host_injection_binding]
name = "ironbank_host_injection_binding"
action = "allow"
detection_level = "informational"
match = 'http.host == "egress.capsem.test" && tcp.port == "443"'
[corp.rules.allow_ironbank_host_injection]
name = "allow_ironbank_host_injection"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Hermetic host credential injection fixture."
match = 'http.host == "egress.capsem.test" && tcp.port == "443" && http.path == "/echo"'
''')
        monkeypatch.setenv("CAPSEM_CORP_CONFIG", str(corp))
        if source == "startup":
            inputs = service.home_dir / "host-input.json"
            inputs.write_text(json.dumps({"credentials": [{
                "provider": "google", "value": secrets["file"], "storage": "file",
            }]}))
            inputs.chmod(0o600)
            monkeypatch.setenv("CAPSEM_CREDENTIAL_INJECTION_FILE", str(inputs))
            monkeypatch.setenv("CAPSEM_CREDENTIAL_INJECTION_ENV", "GOOGLE_API_KEY")
            monkeypatch.setenv("CAPSEM_CREDENTIAL_INJECTION_STORAGE", "memory")
            monkeypatch.setenv("GOOGLE_API_KEY", secrets["memory"])
            monkeypatch.setenv("OPENAI_API_KEY", unselected)
        service.start()
        gateway.start()
        client = TcpHttpClient(gateway.base_url, gateway.token)
        store = service.home_dir / "credential-store.json"
        references = {}
        status, denied = client.call_json("POST", "/credentials/inject", {
            "provider": "google", "value": "capsem_test_unauthenticated", "storage": "memory",
        }, use_auth=False)
        assert status == 401 and "credential_ref" not in denied
        if source == "startup":
            before = client.get("/plugins/credential_broker/credentials/info")
            assert before["store"]["cached_count"] == 2, before
            assert all(value not in json.dumps(before) for value in markers)
        # This is a product workload using only the shipped Python stdlib.
        with image_session(service, service.tmp_dir / "registry", vm_name("host-injection"), client=client) as vm_id:
            assert service.proc is not None
            owners = [child for child in psutil.Process(service.proc.pid).children()
                      if child.name() == "capsem-process"]
            assert len(owners) == 1
            environment = repr(owners[0].environ())
            assert all(value not in environment for value in markers)
            for storage, secret in secrets.items():
                if source == "api":
                    status, response = client.call_json("POST", "/credentials/inject", {
                        "provider": "google", "value": secret, "storage": storage,
                    }, timeout=30)
                    assert status == 200, response
                    assert set(response) == {"credential_ref", "storage"}
                    assert response["storage"] == storage
                    reference = response["credential_ref"]
                else:
                    reference = _credential_reference("google", secret)
                    assert reference in read_log_stream(service.tmp_dir / "service.log")
                assert reference.startswith("credential:blake3:") and len(reference) == 82
                references[storage] = reference
                script = f'''import json, os, urllib.request
request = urllib.request.Request("https://egress.capsem.test/echo",
    data=b"host injection", headers={{"authorization": "Bearer " + {reference!r}}})
with urllib.request.urlopen(request, timeout=30) as response:
    print(json.dumps({{"status": response.status, "echo": json.load(response),
        "environment": dict(os.environ)}}))
'''
                assert all(value not in script for value in markers)
                result = workload_exec(client, vm_id, "python3 -c " + shlex.quote(script))
                assert result["exit_code"] == 0, result
                output = result["stdout_text"]
                assert all(value not in output for value in markers)
                observed = json.loads(output)
                assert observed["status"] == 200
                assert observed["echo"]["has_authorization"] is True
                assert observed["echo"]["authorization_is_broker_ref"] is False
                persisted = store.read_text() if store.exists() else ""
                assert secrets["memory"] not in persisted
                assert unselected not in persisted
                if storage == "file":
                    assert secret in persisted
                    assert stat.S_IMODE(store.stat().st_mode) == 0o600

            records = [json.loads(line) for line in (service.tmp_dir / "upstream.jsonl").read_text().splitlines()]
            # The hermetic transcript also contains DNS records.
            requests = [row for row in records if row.get("path") == "/echo"]
            assert [row["headers"]["authorization"] for row in requests] == [
                "Bearer " + value for value in secrets.values()
            ]
            with closing(_connect_session_db(service, vm_id)) as ledger:
                for table, columns in (
                    ("net_events", EXPECTED_NET_COLUMNS),
                    ("security_rule_events", EXPECTED_SECURITY_COLUMNS),
                    ("substitution_events", EXPECTED_SUBSTITUTION_COLUMNS),
                ):
                    assert _table_columns(ledger, table) == columns
                rows = _eventually(lambda: ledger.execute(
                    "SELECT * FROM net_events WHERE path='/echo' ORDER BY id"
                ).fetchall(), lambda rows: len(rows) == 2)
                assert [row["credential_ref"] for row in rows] == list(references.values())
                assert all(row["decision"] == "allowed" and row["policy_rule"] == "corp.rules.allow_ironbank_host_injection" for row in rows)
                for row in rows:
                    assert row["domain"] == "egress.capsem.test" and row["port"] == 443
                    assert row["method"] == "POST" and row["status_code"] == 200
                    assert row["conn_type"] == "https-mitm"
                    assert row["event_id"] and row["trace_id"]
                    assert all(value not in row["request_headers"] for value in markers)
                injections = ledger.execute(
                    "SELECT provider, substitution_ref, source FROM substitution_events WHERE outcome='injected' ORDER BY id"
                ).fetchall()
                assert [(row["provider"], row["substitution_ref"], row["source"]) for row in injections] == [
                    ("google", reference, "http.header.authorization") for reference in references.values()
                ]
                for table in ("net_events", "security_rule_events", "substitution_events"):
                    serialized = repr([tuple(row) for row in ledger.execute(f"SELECT * FROM {table}")])
                    assert all(value not in serialized for value in markers)
            info = client.get("/plugins/credential_broker/credentials/info")
            assert all(value not in json.dumps(info) for value in markers)
            assert {row["credential_ref"] for row in info["inventory"]} >= set(references.values())
            session = vm_session_dir(service.tmp_dir, client, vm_id)
            status, stopped = client.call_json("POST", f"/vms/{vm_id}/stop", {})
            assert status == 200 and stopped == {"success": True, "persistent": True}
            # Stop awaits the owner, so these streams are flushed before delete.
            for name in ("process.log", "serial.log"):
                text = read_log_stream(session / name)
                assert text, name
                assert all(value not in text for value in markers)
        gateway_log = gateway.stop_and_read_log()
        service_log = service.stop_and_read_log()
        assert all(value not in gateway_log + service_log for value in markers)
        for log in service.tmp_dir.rglob("*.log"):
            assert all(value not in read_log_stream(log) for value in markers)
    finally:
        gateway.stop()
        service.stop()
        stop_process(upstream)
