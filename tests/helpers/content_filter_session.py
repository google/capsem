"""Native product-image fixture shared by content-filter acceptance and collectors."""

from __future__ import annotations

import contextlib
import json
import os
import subprocess
import uuid
from dataclasses import dataclass
from pathlib import Path

from helpers.body_archive import security_payload
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.image_session import WORKSPACE, image_session, workload_exec
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import ServiceInstance, vm_session_db_path
from helpers.session_ledger import stop_session_for_ledger_read
from helpers.uds_client import UdsHttpClient

FIXTURE_HOST = "fixture.capsem.test"
OAUTH_HOST = "oauth2.googleapis.com"


def upstream_records(text: str, *, required_hosts=(FIXTURE_HOST,)) -> list[dict]:
    """Validate DNS exchanges explicitly, then return HTTP rows for byte proofs."""
    dns, http = [], []
    for line in text.splitlines():
        row = json.loads(line)
        if row.get("kind") == "dns":
            assert row["qname"].rstrip(".") in (FIXTURE_HOST, OAUTH_HOST), row
            assert row["qtype"] in (1, 28) and row["qclass"] == 1, row
            assert row["rcode"] == 0 and row["source_proto"] in ("udp", "tcp"), row
            assert row["request_bytes"] > 0 and row["response_bytes"] > 0, row
            dns.append(row)
        else:
            assert "kind" not in row, row
            assert {"method", "path", "headers", "request_bytes"} <= row.keys(), row
            http.append(row)
    observed = {row["qname"].rstrip(".") for row in dns}
    assert set(required_hosts) <= observed, "native requests did not traverse required hermetic DNS hosts"
    return http


def plugin_execution_payloads(conn) -> dict:
    """One forensic payload per event, including events attached to CEL rules."""
    return {
        row[0]: security_payload(conn, row[0], "security_decision_events")
        for row in conn.execute(
            "SELECT DISTINCT event_id FROM main.security_decision_events"
        )
    }


def fixture_config(ready: dict) -> str:
    port = int(ready["http_addr"].rsplit(":", 1)[1])
    return f'''
refresh_policy = "24h"
[network.dns]
upstreams = [{json.dumps(ready["dns_udp_addr"])}]
[network.upstream_overrides."{FIXTURE_HOST}:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"
[network.upstream_overrides."{OAUTH_HOST}:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"
[settings."security.web.http_upstream_ports"]
value = [80, {port}]
modified = "2026-10-23T00:00:00Z"
[settings."vm.resources.log_bodies"]
value = true
modified = "2026-10-23T00:00:00Z"
[settings."vm.resources.max_body_capture"]
value = 8192
modified = "2026-10-23T00:00:00Z"
[ai.google]
allowed_remote_targets = ["{FIXTURE_HOST}:443"]
[ai.google.rules.content_broker_binding]
name = "content_broker_binding"
action = "allow"
detection_level = "informational"
match = 'http.host == "{FIXTURE_HOST}" && tcp.port == "443"'
[corp.rules.allow_content_fixture]
name = "allow_content_fixture"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Admit only the hermetic content-filter and synthetic broker fixtures."
match = 'http.host == "{FIXTURE_HOST}" && tcp.port == "443" && (http.path.matches("^/content/(echo|text|stream)$") || (http.path == "/echo"))'
[corp.rules.allow_google_oauth_fixture]
name = "allow_google_oauth_fixture"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Canonical Google provider identity routed only to the hermetic token fixture."
match = 'http.host == "{OAUTH_HOST}" && tcp.port == "443" && http.path == "/oauth/token"'

'''


@dataclass
class FilterSession:
    service: ServiceInstance
    client: UdsHttpClient
    gateway: TcpHttpClient
    vm_id: str
    transcript: Path
    mock_process: subprocess.Popen[str]
    corp_config: Path

    def python(self, script: str) -> dict:
        name = f"content-proof-{uuid.uuid4().hex[:8]}.py"
        uploaded = self.client.post_bytes(
            f"/vms/{self.vm_id}/files/content?path={name}", script.encode(), timeout=30
        )
        assert uploaded["success"] is True, uploaded
        result = workload_exec(self.client, self.vm_id, f"python3 {WORKSPACE}/{name}", timeout=120)
        assert result["exit_code"] == 0, result
        lines = [
            line for line in result["stdout_text"].splitlines() if line.startswith("FILTER_RESULT=")
        ]
        assert len(lines) == 1, result
        return json.loads(lines[0].split("=", 1)[1])

    def upstream(self, *, required_hosts=(FIXTURE_HOST,)) -> list[dict]:
        return upstream_records(self.transcript.read_text(), required_hosts=required_hosts)

    def ledger(self) -> Path:
        path = vm_session_db_path(self.service.tmp_dir, self.client, self.vm_id)
        return stop_session_for_ledger_read(self.client, self.vm_id, path)


@contextlib.contextmanager
def filter_session(workdir: Path, plugin: str, mode: str, *, content=None, capture_requests=True):
    """Configure public controls before boot; use exactly the verified product dev pin."""
    workdir.mkdir(parents=True, exist_ok=True)
    transcript = workdir / "upstream.jsonl"
    mock, ready = start_mock_server(
        addr="127.0.0.1:0",
        request_log=transcript if capture_requests else None,
        capture_requests=capture_requests,
        dns_answers="routable",
    )
    service = ServiceInstance()
    gateway = None
    previous = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        corp = workdir / "corp.toml"
        corp.write_text(fixture_config(ready))
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp)
        service.start()
        client = service.client()
        gateway = GatewayInstance(service.uds_path)
        gateway.start()
        public = TcpHttpClient(gateway.base_url, gateway.token)
        patch = {"mode": mode}
        if content is not None:
            patch["content"] = content
        result = public.patch(f"/plugins/{plugin}/edit", patch, timeout=30)
        assert result["id"] == plugin and result["config"]["mode"] == mode, result
        assert client.get(f"/plugins/{plugin}/info")["config"] == result["config"]
        with image_session(
            service, workdir / "image", f"content-proof-{uuid.uuid4().hex[:8]}"
        ) as vm_id:
            yield FilterSession(service, client, public, vm_id, transcript, mock, corp)
    finally:
        if gateway is not None:
            gateway.stop()
        service.stop()
        stop_process(mock)
        if previous is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = previous
