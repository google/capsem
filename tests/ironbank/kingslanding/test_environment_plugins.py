"""Inspect and exercise the environment plugins used by native sandboxes."""

import json
import secrets
import shlex
import uuid
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance, exec_output_text, vm_name
from helpers.service_gateway import ServiceGateway

from build_system.tests.helpers.mock_server import MOCK_SERVER_ADDR, _acquire_lock
from tests.ironbank.kingslanding.test_environment_startup import create, request, save

pytestmark = pytest.mark.integration


def test_plugin_views_bind_the_saved_revision_and_report_whole_corporate_overrides(monkeypatch, tmp_path):
    corp = tmp_path / "corp.toml"
    corp.write_text('refresh_policy = "24h"\n')
    monkeypatch.setenv("CAPSEM_CORP_CONFIG", str(corp))
    service = ServiceInstance()
    gateway = ServiceGateway(service)
    defaults_route = "/environments/plugins/list"
    try:
        service.start()
        saved = save(service, {
            "name": "plugin-inspection",
            "plugins": {"pii_filter": {"outbound": {"mode": "rewrite", "options": {"email": False}}}},
        })
        other = save(service, {"name": "separate-plugin-inspection"})
        gateway.start()
        route = f"/environments/{saved['environment_id']}/plugins/list"
        exact = f"{route}?revision_id={saved['revision_id']}"
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            status, result = operator.call_json("GET", exact)
            assert status == 200, result
            assert result["revision"] == saved
            entries = {item["descriptor"]["id"]: item for item in result["plugins"]}
            assert len(entries) == len(result["plugins"]) == 7
            assert entries["pii_filter"]["source"] == "environment"
            assert entries["pii_filter"]["effective"]["outbound.options.email"] is False
            assert entries["credential_broker"]["descriptor"]["required"] is True
            assert entries["credential_broker"]["configured"] == {}
            corp.write_text('''refresh_policy = "24h"
[environment_plugins.pii_filter.outbound]
mode = "block"
detection_level = "high"
[environment_plugins.utf8_guard]
mode = "block"
[environment_plugins.utf8_guard.options]
outbound = false
''')
            result = operator.get(exact)
            entries = {item["descriptor"]["id"]: item for item in result["plugins"]}
            pii = entries["pii_filter"]
            assert pii["source"] == "corporate" and pii["locked"] is True
            assert pii["configured"]["outbound.mode"] == "rewrite"
            assert pii["configured"]["outbound.options.email"] is False
            assert pii["effective"]["outbound.mode"] == "block"
            assert pii["effective"]["outbound.options.email"] is True
            assert pii["directions"][1] == {
                "direction": "outbound", "mode": "block", "detection_level": "high"
            }
            assert entries["utf8_guard"]["directions"][1] == {
                "direction": "outbound", "mode": "disable", "detection_level": None
            }
            defaults = operator.get(defaults_route)
            assert defaults["revision"] is None
            assert all(item["configured"] == {} for item in defaults["plugins"])
            assert operator.call_json("GET", f"{route}?revision_id=invalid")[0] == 400
            assert operator.call_json("GET", f"{route}?revision_id={other['revision_id']}")[0] == 409

            secret = secrets.token_hex(32)
            status, grant = operator.call_json("POST", "/agent-access", {
                "request_id": str(uuid.uuid4()),
                "ownership_token": secret,
                "lease_seconds": 300,
                "scope": {"operations": ["inspect"], "environment_ids": [saved["environment_id"]]},
            })
            assert status == 200, grant
            with closing(TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")) as agent:
                assert agent.call_json("GET", exact) == (200, result)
                assert agent.call_json("GET", defaults_route)[0] == 403
                assert agent.call_json("GET", f"/environments/{other['environment_id']}/plugins/list")[0] == 403
    finally:
        gateway.stop()
        service.stop()


def test_native_pii_filter_rewrites_only_the_saved_direction_in_each_sandbox(monkeypatch, tmp_path):
    submitted = {"contact": "sender@example.com", "category": "unchanged"}
    returned = {"contact": "recipient@example.com", "category": "unchanged"}
    received = []

    class Upstream(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            received.append((self.path, json.loads(body)))
            payload = json.dumps(returned).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_):
            pass

    # Use the integration fixture's intercepted guest HTTP port and host lock.
    host, port_text = MOCK_SERVER_ADDR.rsplit(":", 1)
    port = int(port_text)
    with _acquire_lock(MOCK_SERVER_ADDR), ThreadingHTTPServer((host, port), Upstream) as upstream:
        corp = tmp_path / "direction-policy.toml"
        corp.write_text(f'''refresh_policy = "24h"
[settings."security.web.http_upstream_ports"]
value = [{port}]
modified = "2026-06-14T00:00:00Z"
[corp.rules.allow_plugin_fixture]
name = "allow_plugin_fixture"
action = "allow"
priority = -100
match = 'http.host == "127.0.0.1" && tcp.port == "{port}" && http.path == "/contact"'
''')
        monkeypatch.setenv("CAPSEM_CORP_CONFIG", str(corp))
        service = ServiceInstance()
        worker = Thread(target=upstream.serve_forever, daemon=True)
        worker.start()
        try:
            service.start()
            sandboxes = []
            for direction in ("outbound", "inbound"):
                configuration = {side: {"mode": "disable"} for side in ("outbound", "inbound")}
                configuration[direction] = {"mode": "rewrite", "options": {"email": True}}
                saved = save(service, {
                    "name": f"{direction}-plugin",
                    "plugins": {"pii_filter": configuration},
                })
                vm = create(service, saved, vm_name(f"{direction}-http-plugin"))
                assert request(service, "GET", f"/vms/{vm}/info")["environment"]["boot"] == saved
                sandboxes.append(vm)
            probe = f"""import urllib.request
request = urllib.request.Request(
    "http://127.0.0.1:{port}/contact",
    data={json.dumps(submitted).encode()!r},
    headers={{"Content-Type": "application/json"}},
)
with urllib.request.urlopen(request, timeout=10) as response:
    print(response.read().decode())
"""
            responses = []
            for vm in sandboxes:
                result = request(service, "POST", f"/vms/{vm}/exec", {
                    "command": f"python3 -c {shlex.quote(probe)}", "target": "vm", "timeout_secs": 15,
                })
                assert result["exit_code"] == 0, result
                responses.append(json.loads(exec_output_text(result)))

            assert received == [
                ("/contact", {**submitted, "contact": "*" * len(submitted["contact"])}),
                ("/contact", submitted),
            ]
            assert responses == [
                returned,
                {**returned, "contact": "*" * len(returned["contact"])},
            ]
        finally:
            service.stop()
            upstream.shutdown()
            worker.join()
