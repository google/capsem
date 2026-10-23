"""Native content-filter controls, delivered bytes and durable leak prevention.

Synthetic fixture material only. Each case traverses the product dev workload,
TLS interception, the real plugin pipeline and a hermetic upstream.
"""

from __future__ import annotations

import base64
import sqlite3
from contextlib import closing

import pytest
from helpers.body_archive import archived_bodies
from helpers.content_filter_session import (
    FIXTURE_HOST,
    OAUTH_HOST,
    filter_session,
    plugin_execution_payloads,
)
from helpers.session_ledger import open_session_ledger
from ironbank.test_http_protocol_ledger import EXPECTED_NET_COLUMNS

pytestmark = pytest.mark.integration
EMAIL = b"demo.person@example.test"
TOKEN = b"ghp_0123456789abcdefghijklmnopqrstuvwxyz"


def request_script(
    body: bytes,
    *,
    media="text/plain",
    query="",
    chunked=False,
    compress=False,
    headers=None,
    split_at=5,
) -> str:
    return f"""
import base64, gzip, http.client, json
body = base64.b64decode({base64.b64encode(body).decode()!r})
headers = {{"content-type": {media!r}, **{headers or {}!r}}}
if {compress!r}:
    body = gzip.compress(body)
    headers["content-encoding"] = "gzip"
conn = http.client.HTTPSConnection({FIXTURE_HOST!r}, timeout=30)
parts = [body[:{split_at}], body[{split_at}:]] if {chunked!r} else body
conn.request("POST", "/content/echo{query}", body=parts, headers=headers, encode_chunked={chunked!r})
response = conn.getresponse()
data = response.read()
if response.getheader("content-encoding") == "gzip":
    data = gzip.decompress(data)
print("FILTER_RESULT=" + json.dumps({{"status": response.status, "body": base64.b64encode(data).decode()}}))
conn.close()
"""


def assert_private_ledger(
    env, plugin: str, *, forbidden: tuple[bytes, ...], detections: int
) -> None:
    path = env.ledger()
    with closing(open_session_ledger(path)) as conn:
        conn.row_factory = sqlite3.Row
        nets = [
            dict(row)
            for row in conn.execute(
                "SELECT * FROM net_events WHERE path LIKE '/content/%'"
            )
        ]
        assert nets, "the real HTTP path emitted no network ledger"
        for row in nets:
            assert set(row) == EXPECTED_NET_COLUMNS, row
            assert row["trace_id"] and row["event_id"]
            for value in row.values():
                for raw in forbidden:
                    assert raw.decode(errors="backslashreplace") not in str(value)
        payloads = plugin_execution_payloads(conn)
        matched_events = {
            event_id
            for event_id, payload in payloads.items()
            if any(
                execution["plugin_id"] == plugin and execution["applied"]
                for execution in payload.get("plugin_executions", [])
            )
        }
        executions = [
            execution
            for payload in payloads.values()
            for execution in payload.get("plugin_executions", [])
            if execution["plugin_id"] == plugin
        ]
        assert executions, payloads
        assert sum(bool(entry["applied"]) for entry in executions) == detections
        for row in nets:
            if row["event_id"] in matched_events:
                for column in (
                    "request_body_preview",
                    "response_body_preview",
                    "request_headers",
                    "response_headers",
                ):
                    assert row[column] is None, row
        info = env.client.get(f"/plugins/{plugin}/info")
        public = env.gateway.get(f"/plugins/{plugin}/info")
        assert info["runtime"] == public["runtime"]
        assert info["runtime"]["applied_count"] == detections
        assert info["runtime"]["detection_count"] == detections
    for table, event, direction, body in archived_bodies(path):
        assert not (
            table == "net_events"
            and event in matched_events
            and direction in {"request", "response"}
        ), "matching HTTP materialization was retained (including compressed bytes)"
        for raw in forbidden:
            assert raw not in body, "sensitive fixture leaked to the body archive"
    log = env.service.stop_and_read_log()
    for raw in forbidden:
        assert raw.decode(errors="backslashreplace") not in log, (
            "sensitive fixture leaked to service logs"
        )


@pytest.mark.parametrize(
    "direction,compressed,chunked",
    [("outbound", False, True), ("inbound", False, False), ("both", True, False)],
)
def test_pii_rewrite_uses_actual_bytes_and_never_archives_matches(
    tmp_path, direction, compressed, chunked
):
    with filter_session(
        tmp_path, "pii_filter", "rewrite", content={"direction": direction}
    ) as env:
        # Match beyond the configured 8 KiB capture window as well as the
        # short compressed-response case: enforcement must inspect wire bytes.
        prefix = b"safe-prefix " * 800 if direction == "outbound" else b""
        body = prefix + EMAIL
        result = env.python(
            request_script(
                body,
                query="?gzip=1" if direction == "inbound" else "",
                chunked=chunked,
                compress=compressed,
            )
        )
        assert result == {
            "status": 200,
            "body": base64.b64encode(prefix + b"*" * len(EMAIL)).decode(),
        }
        upstream = [row for row in env.upstream() if row["path"] == "/content/echo"]
        assert len(upstream) == 1, upstream
        assert upstream[0]["request_body"] == (
            body.decode()
            if direction == "inbound"
            else prefix.decode() + "*" * len(EMAIL)
        )
        assert upstream[0]["request_bytes"] == len(body)
        if compressed:
            assert "content-encoding" not in upstream[0]["headers"]
        assert_private_ledger(env, "pii_filter", forbidden=(EMAIL,), detections=1)


def test_secret_body_and_custom_header_are_blocked_before_upstream(tmp_path):
    with filter_session(tmp_path, "secret_exfiltration", "block") as env:
        result = env.python(request_script(TOKEN, chunked=True))
        assert result["status"] == 403, result
        header = env.python(
            request_script(b"safe body", headers={"x-leaked-token": TOKEN.decode()})
        )
        assert header["status"] == 403, header
        assert not [row for row in env.upstream() if row["path"] == "/content/echo"]
        assert_private_ledger(
            env, "secret_exfiltration", forbidden=(TOKEN,), detections=2
        )


@pytest.mark.parametrize("direction", ["outbound", "inbound"])
def test_utf8_guard_rejects_truncation_but_preserves_unicode_and_binary(
    tmp_path, direction
):
    with filter_session(
        tmp_path, "utf8_guard", "block", content={"direction": direction}
    ) as env:
        unicode = "hello 🌍 café 漢字".encode()
        valid = env.python(request_script(unicode, chunked=True, split_at=8))
        assert valid == {"status": 200, "body": base64.b64encode(unicode).decode()}
        binary = b"\xff\xfe\xe2\x82"
        valid_binary = env.python(
            request_script(binary, media="application/octet-stream")
        )
        assert valid_binary == {
            "status": 200,
            "body": base64.b64encode(binary).decode(),
        }
        rejected = env.python(request_script(b"truncated:\xe2\x82", chunked=True))
        assert rejected["status"] == 403, rejected
        upstream = [row for row in env.upstream() if row["path"] == "/content/echo"]
        assert len(upstream) == (3 if direction == "inbound" else 2), upstream
        assert upstream[0]["request_body"] == unicode.decode()
        assert_private_ledger(
            env, "utf8_guard", forbidden=(b"truncated:",), detections=1
        )


def test_secret_guard_preserves_authorized_broker_header_and_blocks_reference_copy(
    tmp_path,
):
    with filter_session(tmp_path, "secret_exfiltration", "block") as env:
        base = f"https://{FIXTURE_HOST}"
        result = env.python(f"""
import json, urllib.request, urllib.parse, urllib.error
base = {base!r}
request = urllib.request.Request("https://{OAUTH_HOST}/oauth/token", data=urllib.parse.urlencode({{"code": "capsem_test_oauth_code_content_proof"}}).encode(), headers={{"content-type": "application/x-www-form-urlencoded"}})
with urllib.request.urlopen(request, timeout=30) as response:
    token = json.load(response)["access_token"]
assert token.startswith("credential:blake3:"), token
request = urllib.request.Request(base + "/echo", data=b"authorized broker traffic", headers={{"authorization": "Bearer " + token, "content-type": "text/plain"}})
with urllib.request.urlopen(request, timeout=30) as response:
    echo = json.load(response)
request = urllib.request.Request("https://{FIXTURE_HOST}/content/echo", data=token.encode(), headers={{"content-type": "text/plain"}})
try:
    urllib.request.urlopen(request, timeout=30)
    raise AssertionError("reference copy was forwarded")
except urllib.error.HTTPError as error:
    status = error.code
print("FILTER_RESULT=" + json.dumps({{"status": status, "has_authorization": echo["has_authorization"], "ref_at_upstream": echo["authorization_is_broker_ref"]}}))
""")
        assert result == {
            "status": 403,
            "has_authorization": True,
            "ref_at_upstream": False,
        }
        upstream = env.upstream(required_hosts=(FIXTURE_HOST, OAUTH_HOST))
        assert len([row for row in upstream if row["path"] == "/oauth/token"]) == 1
        assert len([row for row in upstream if row["path"] == "/echo"]) == 1
        assert not [row for row in upstream if row["path"] == "/content/echo"]
        assert_private_ledger(
            env,
            "secret_exfiltration",
            forbidden=(b"capsem_test_oauth_access_",),
            detections=1,
        )
