"""Configured and effective plugins use the current corporate policy on the real gateway."""

import secrets
import uuid
from contextlib import closing

import pytest
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance
from helpers.service_gateway import ServiceGateway

from tests.ironbank.kingslanding.test_environment_startup import save

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
