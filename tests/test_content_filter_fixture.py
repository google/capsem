"""Source contracts for native filter stimuli; these do not claim VM acceptance."""

from __future__ import annotations

import ast
import json
import tomllib

import pytest
from helpers.content_filter_session import (
    FIXTURE_HOST,
    OAUTH_HOST,
    fixture_config,
    upstream_records,
)
from ironbank.test_content_filter_ledger import request_script


def test_hermetic_content_config_uses_existing_public_settings_and_scoped_rules():
    ready = {"dns_udp_addr": "127.0.0.1:42000", "http_addr": "127.0.0.1:43000"}
    config = tomllib.loads(fixture_config(ready))
    assert config["network"] == {
        "dns": {"upstreams": [ready["dns_udp_addr"]]},
        "upstream_overrides": {
            f"{FIXTURE_HOST}:443": {"dial": ready["http_addr"], "protocol": "http"},
            f"{OAUTH_HOST}:443": {"dial": ready["http_addr"], "protocol": "http"}
        },
    }
    settings = config["settings"]
    assert settings["security.web.http_upstream_ports"]["value"] == [80, 43000]
    assert settings["vm.resources.log_bodies"]["value"] is True
    assert settings["vm.resources.max_body_capture"]["value"] == 8192
    assert config["ai"]["google"]["allowed_remote_targets"] == [f"{FIXTURE_HOST}:443"]
    binding = config["ai"]["google"]["rules"]["content_broker_binding"]
    assert binding["match"] == f'http.host == "{FIXTURE_HOST}" && tcp.port == "443"'
    rule = config["corp"]["rules"]["allow_content_fixture"]
    assert rule["action"] == "allow"
    assert rule["priority"] == -100
    assert "/content/(echo|text|stream)" in rule["match"]
    assert 'http.path == "/echo"' in rule["match"]
    assert "/oauth/token" not in rule["match"]
    oauth = config["corp"]["rules"]["allow_google_oauth_fixture"]
    assert oauth["action"] == "allow"
    assert oauth["match"] == f'http.host == "{OAUTH_HOST}" && tcp.port == "443" && http.path == "/oauth/token"'


def test_dns_exchanges_are_checked_separately_from_http_records():
    dns = {
        "kind": "dns",
        "qname": FIXTURE_HOST,
        "qtype": 1,
        "qclass": 1,
        "rcode": 0,
        "source_proto": "udp",
        "request_bytes": 37,
        "response_bytes": 53,
    }
    http = {"method": "POST", "path": "/content/echo", "headers": {}, "request_bytes": 3}
    transcript = "\n".join(json.dumps(row) for row in (dns, http))
    assert upstream_records(transcript) == [http]
    with pytest.raises(AssertionError, match="DNS"):
        upstream_records(transcript, required_hosts=(FIXTURE_HOST, OAUTH_HOST))
    oauth = dict(dns, qname=OAUTH_HOST)
    assert upstream_records(transcript + "\n" + json.dumps(oauth), required_hosts=(FIXTURE_HOST, OAUTH_HOST)) == [http]


@pytest.mark.parametrize(
    "bad",
    [
        {"kind": "unexpected"},
        {"kind": "dns", "qname": "unrelated.test"},
        {"method": "POST"},
    ],
)
def test_upstream_records_never_silently_discard_unknown_or_malformed_rows(bad):
    with pytest.raises((AssertionError, KeyError)):
        upstream_records(json.dumps(bad))


def test_missing_or_failed_dns_is_not_accepted_as_a_successful_native_path():
    with pytest.raises(AssertionError, match="DNS"):
        upstream_records('{"method":"POST","path":"/content/echo","headers":{},"request_bytes":1}')
    failed = {
        "kind": "dns",
        "qname": FIXTURE_HOST,
        "qtype": 1,
        "qclass": 1,
        "rcode": 3,
        "source_proto": "udp",
        "request_bytes": 37,
        "response_bytes": 37,
    }
    with pytest.raises(AssertionError):
        upstream_records(json.dumps(failed))


def test_guest_stimuli_remain_valid_python_for_binary_gzip_and_chunked_cases():
    for body in (b"demo.person@example.test", b"\xff\xe2\x82", "🌍 café 漢字".encode()):
        for options in (
            {"chunked": True},
            {"compress": True},
            {"media": "application/octet-stream"},
            {"headers": {"x-leaked-token": "synthetic"}},
        ):
            ast.parse(request_script(body, **options))


def test_applied_receipts_include_rule_backed_events_once(monkeypatch):
    import sqlite3

    from helpers import content_filter_session as fixture

    conn = sqlite3.connect(":memory:")
    conn.executescript("""
        CREATE TABLE security_decision_events(event_id TEXT, actor TEXT);
        INSERT INTO security_decision_events VALUES
            ('clean-dns', 'security.plugins'),
            ('applied-http', 'corp.rules.allow_content_fixture'),
            ('applied-http', 'profiles.rules.ai_google_content_broker_binding');
    """)
    seen = []

    def read_payload(db, event, table):
        assert db is conn and table == "security_decision_events"
        seen.append(event)
        return {"plugin_executions": [{"plugin_id": "pii_filter", "applied": event == "applied-http"}]}

    monkeypatch.setattr(fixture, "security_payload", read_payload)
    try:
        payloads = fixture.plugin_execution_payloads(conn)
        assert set(payloads) == {"clean-dns", "applied-http"}
        assert len(seen) == 2
        assert sum(row["applied"] for payload in payloads.values() for row in payload["plugin_executions"]) == 1
    finally:
        conn.close()


def test_secret_stimulus_imports_no_literal_but_sends_exact_header(monkeypatch, tmp_path):
    import http.client
    import runpy

    from ironbank.test_content_filter_ledger import TOKEN

    script = request_script(b"safe body", headers={"x-leaked-token": TOKEN.decode()})
    assert TOKEN not in script.encode(), "the uploaded source must not create its own archive leak"
    requests = []

    class Connection:
        status = 403

        def __init__(self, host, *, timeout):
            assert host == FIXTURE_HOST and timeout == 30

        def request(self, method, path, **kwargs):
            requests.append((method, path, kwargs))

        def getresponse(self):
            return self

        def read(self):
            return b""

        def getheader(self, _name):
            return None

        def close(self):
            pass

    monkeypatch.setattr(http.client, "HTTPSConnection", Connection)
    uploaded = tmp_path / "synthetic-content-stimulus.py"
    uploaded.write_text(script)
    runpy.run_path(str(uploaded))
    assert len(requests) == 1
    assert requests[0][2]["body"] == b"safe body"
    assert requests[0][2]["headers"]["x-leaked-token"].encode() == TOKEN


@pytest.mark.parametrize("invalid", ["raw-token", "malformed-reference", "truncated"])
def test_public_capture_reference_requires_complete_redacted_archive(invalid):
    from helpers.content_filter_session import captured_access_token_if_ready

    reference = "credential:blake3:" + "a" * 64
    access = {"raw-token": "capsem_test_oauth_access_synthetic",
              "malformed-reference": "credential:blake3:not-a-digest"}.get(invalid, reference)
    body = {"source_table": "net_events", "direction": "response", "encoding": "utf8",
            "truncated": invalid == "truncated", "truncated_for_transport": False,
            "content": json.dumps({"access_token": access})}

    class Client:
        phase = 0

        def get(self, path):
            if path == "/vms/vm/stats/detail":
                return {"http_events": [] if self.phase == 0 else [{"domain": OAUTH_HOST,
                    "path": "/oauth/token", "status_code": 200, "event_id": "oauth-event"}]}
            assert path == "/vms/vm/bodies/oauth-event"
            return {"bodies": [] if self.phase == 1 else [body]}

    client = Client()
    assert captured_access_token_if_ready(client, "vm") is None
    client.phase = 1
    assert captured_access_token_if_ready(client, "vm") is None
    client.phase = 2
    with pytest.raises(AssertionError):
        captured_access_token_if_ready(client, "vm")
    body["truncated"] = False
    body["content"] = json.dumps({"access_token": reference})
    assert captured_access_token_if_ready(client, "vm") == reference
