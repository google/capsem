"""Native TCP DNS keeps one original query behind session-scoped review."""

import json
import textwrap
from concurrent.futures import ThreadPoolExecutor

import pytest
from helpers.service import exec_output_text

from tests.ironbank.approval_fixture import (
    assert_original_cancelled,
    pending_original_operation,
    resolve_original_operation,
)
from tests.ironbank.approval_native import (
    assert_approval_audit,
    native_approval_session,
    upstream_records,
)

pytestmark = pytest.mark.integration
QNAME = "fixture.capsem.test"
POLICY = """
[corp.rules.ask_native_dns]
name = "ask_native_dns"
action = "ask"
priority = -100
match = 'dns.qname == "fixture.capsem.test"'
"""


def dns_script(cancel=False):
    end = (
        "time.sleep(5); print('DNS_ORIGINAL_CLOSED')"
        if cancel
        else """
        def exact(size):
            data = b''
            while len(data) < size:
                chunk = sock.recv(size-len(data))
                assert chunk, 'DNS connection closed before complete response'
                data += chunk
            return data
        reply = exact(struct.unpack('!H', exact(2))[0])
        identifier, flags, questions, answers, _, _ = struct.unpack('!HHHHHH', reply[:12])
        assert identifier == 0x7531 and questions == 1
        print('DNS_APPROVAL_RESULT=' + json.dumps({'rcode': flags & 15, 'answers': answers}))
    """
    )
    return (
        textwrap.dedent(f"""
        import json, socket, struct, time
        server = next(line.split()[1] for line in open('/etc/resolv.conf') if line.startswith('nameserver '))
        labels = b''.join(bytes([len(part)]) + part.encode() for part in {QNAME!r}.split('.'))
        packet = struct.pack('!HHHHHH', 0x7531, 0x0100, 1, 0, 0, 0) + labels + b'\\0' + struct.pack('!HH',1,1)
        with socket.create_connection((server,53), timeout=15) as sock:
            sock.sendall(struct.pack('!H',len(packet)) + packet)
    """).strip()
        + "\n"
        + textwrap.indent(textwrap.dedent(end).strip(), "    ")
    )


def dns_rows(ready):
    return [
        row for row in upstream_records(ready) if row.get("kind") == "dns" and row["qname"] == QNAME
    ]


@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_dns_tcp_original_query_resolves_once(decision):
    with native_approval_session(POLICY) as (service, client, api, vm_id, ready):
        uploaded = client.upload_file(vm_id, "approval-dns.py", dns_script())
        assert uploaded["success"]

        def no_effect():
            assert dns_rows(ready) == [], "unresolved original DNS query reached upstream"

        result, receipt = resolve_original_operation(
            lambda: client.post(
                f"/vms/{vm_id}/exec",
                {
                    "command": f"python3 {uploaded['vm_path']}",
                    "target": "vm",
                    "timeout_secs": 30,
                },
                timeout=40,
            ),
            api,
            vm_id,
            decision,
            no_effect,
            None,
        )
        assert result["exit_code"] == 0, result
        lines = [
            line.split("=", 1)[1]
            for line in exec_output_text(result).splitlines()
            if line.startswith("DNS_APPROVAL_RESULT=")
        ]
        assert len(lines) == 1
        reply = json.loads(lines[0])
        assert reply["rcode"] == (0 if decision == "approve" else 3), reply
        assert reply["answers"] == (1 if decision == "approve" else 0), reply
        assert len(dns_rows(ready)) == (1 if decision == "approve" else 0)
        assert_approval_audit(service, client, vm_id, receipt, table="dns_queries")


def test_dns_tcp_disconnect_cancels_original_query():
    with native_approval_session(POLICY) as (service, client, api, vm_id, ready):
        uploaded = client.upload_file(vm_id, "approval-dns-close.py", dns_script(cancel=True))
        assert uploaded["success"]
        with ThreadPoolExecutor(max_workers=1) as executor:
            original = executor.submit(
                client.post,
                f"/vms/{vm_id}/exec",
                {
                    "command": f"python3 {uploaded['vm_path']}",
                    "target": "vm",
                    "timeout_secs": 30,
                },
                timeout=40,
            )
            listing = pending_original_operation(api, vm_id, original, timeout=3)
            assert dns_rows(ready) == []
            result = original.result(timeout=15)
            assert result["exit_code"] == 0 and "DNS_ORIGINAL_CLOSED" in exec_output_text(result), (
                result
            )
        receipt = assert_original_cancelled(api, vm_id, listing)
        assert dns_rows(ready) == []
        assert_approval_audit(service, client, vm_id, receipt, cancelled=True)
