"""Container internet egress goes through the VM's interception: the same
DNS and HTTP policy applies, the same ledger records it, and the container
reaches nothing else in the VM."""

import json
import os
import subprocess
import textwrap
import uuid

import pytest
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import ServiceInstance

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import command, environment

pytestmark = pytest.mark.integration

# Resolves to a routable test address in the mock DNS; `fixture.capsem.test`
# answers 127.0.0.1, which a container would route to its own loopback.
ALLOWED_HOST = "egress.capsem.test"


@pytest.fixture
def egress(tmp_path):
    """A private service whose corp config routes ALLOWED_HOST to the mock
    upstream, blocks one HTTP path and one DNS zone."""
    blocked_zone = f"{uuid.uuid4().hex[:12]}.attacker.test"
    mock, ready = start_mock_server(request_log=tmp_path / "upstream-transcript.jsonl")
    instance = ServiceInstance()
    corp = instance.tmp_dir / "corp.toml"
    corp.write_text(
        textwrap.dedent(
            f"""
            refresh_policy = "24h"

            [network.dns]
            upstreams = [{json.dumps(ready["dns_udp_addr"])}]

            [network.upstream_overrides."{ALLOWED_HOST}:443"]
            dial = {json.dumps(ready["http_addr"])}
            protocol = "http"

            [network.upstream_overrides."{ALLOWED_HOST}:80"]
            dial = {json.dumps(ready["http_addr"])}
            protocol = "http"

            [corp.rules.block_container_secret_path]
            name = "block_container_secret_path"
            action = "block"
            priority = -100
            detection_level = "high"
            reason = "Kingslanding: HTTP policy applies inside containers."
            match = 'http.host == "{ALLOWED_HOST}" && http.path == "/secret"'

            [corp.rules.block_container_dns_exfil]
            name = "block_container_dns_exfil"
            action = "block"
            priority = -100
            detection_level = "high"
            reason = "Kingslanding: DNS policy applies inside containers."
            match = 'dns.qname.matches("(^|.*\\\\.)attacker\\\\.test$")'
            """
        ).strip()
        + "\n"
    )
    previous = os.environ.get("CAPSEM_CORP_CONFIG")
    os.environ["CAPSEM_CORP_CONFIG"] = str(corp)
    try:
        instance.start()
        yield {"service": instance, "ready": ready, "blocked_zone": blocked_zone}
    finally:
        if previous is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = previous
        try:
            for row in instance.client().get("/vms/list")["sandboxes"]:
                instance.client().delete(f"/vms/{row['id']}/delete")
        finally:
            (tmp_path / "service.log").write_bytes((instance.tmp_dir / "service.log").read_bytes())
            instance.stop()
            stop_process(mock)


def _results(stdout):
    return dict(line.split("=", 1) for line in stdout.splitlines() if "=" in line and " " not in line)


def test_container_egress_is_intercepted_policed_and_audited(egress, tmp_path):
    service = egress["service"]
    client = service.client()
    blocked_zone = egress["blocked_zone"]
    # Each probe prints `name=exit_code`; bodies go to stdout before it.
    script = " ; ".join(
        [
            f"wget -q -T 15 -O - http://{ALLOWED_HOST}/html/about; echo http=$?",
            f"wget -q -T 15 -O - https://{ALLOWED_HOST}/html/about; echo https=$?",
            f"wget -q -T 15 -O - https://{ALLOWED_HOST}/secret; echo secret=$?",
            f"wget -q -T 15 -O - http://probe.{blocked_zone}/; echo dns=$?",
            # busybox nc: exit 0 only if the connection was accepted.
            "echo x | nc -w 3 10.0.1.1 5008; echo escape_gateway=$?",
            "echo x | nc -w 3 10.0.0.1 10443; echo escape_vm=$?",
            "echo x | nc -w 3 10.0.0.1 1053; echo escape_dns=$?",
        ]
    )
    with registry(tmp_path) as (reference, certificate, _):
        run = subprocess.run(
            [*command(service, reference, certificate), "sh", "-c", script],
            env=environment(service),
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
    (tmp_path / "stdout").write_text(run.stdout)
    (tmp_path / "stderr").write_text(run.stderr)
    print(f"EGRESS EVIDENCE: {tmp_path}")
    results = _results(run.stdout)

    assert results.get("http") == "0", run.stdout + run.stderr
    assert results.get("https") == "0", "container must trust the Capsem CA"
    assert run.stdout.count("Capsem mock server about page") == 2, run.stdout
    assert results.get("secret") != "0", "HTTP policy must apply inside the container"
    assert results.get("dns") != "0", "DNS policy must apply inside the container"
    for probe in ("escape_gateway", "escape_vm", "escape_dns"):
        assert results.get(probe) == "1", f"{probe}: container reached a VM service directly"

    rows = client.get("/vms/list")["sandboxes"]
    assert len(rows) == 1
    vm_id = rows[0]["id"]
    # One row per matched rule; the row's rule_id/rule_action are the decision
    # of record (event_json.decision is not applied on HTTP/DNS rows, #203).
    latest = client.get(f"/vms/{vm_id}/security/latest?limit=2000")
    http = []
    dns = []
    for row in latest:
        event = json.loads(row["event_json"])
        if row["event_type"] == "http.request":
            http.append((event["http"]["host"], event["http"]["path"], row["rule_id"], row["rule_action"]))
        elif row["event_type"] == "dns.query":
            dns.append((event["dns"]["qname"], row["rule_id"], row["rule_action"]))
    # The transcript also records DNS exchanges, which have no path.
    transcript = [
        record["path"]
        for record in map(json.loads, (tmp_path / "upstream-transcript.jsonl").read_text().splitlines())
        if "path" in record
    ]
    evidence = f"probes={results} http={http} dns={dns} upstream={transcript}"
    assert any(
        host == ALLOWED_HOST and path == "/html/about" and action == "allow" for host, path, _, action in http
    ), evidence
    assert (ALLOWED_HOST, "/secret", "corp.rules.block_container_secret_path", "block") in http, evidence
    assert any(
        qname.endswith(blocked_zone) and rule == "corp.rules.block_container_dns_exfil" and action == "block"
        for qname, rule, action in dns
    ), evidence
    assert transcript.count("/html/about") == 2, evidence
    assert "/secret" not in transcript, f"a blocked request reached the upstream: {evidence}"
