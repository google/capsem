"""SCM_RIGHTS queues do not own connected sockets reliably on Darwin."""

from __future__ import annotations

import ast
import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
ADOPTION_RATIONALE = """\
Darwin can flush a connected socket referenced only by queued SCM_RIGHTS.
Keep its original owner through a matching receiver adoption receipt, then
release it promptly so ordinary disconnects still propagate. sendmsg completion
is not adoption. See skills/dev-rust-patterns/SKILL.md, Host Unix boundary.
"""

# These are the actual owned handoff paths that regressed, not a count of drops.
HANDOFFS = [
    ("capsem-process/src/cables.rs", "async fn take_within", "stream", "decode_seat_frame"),
    ("capsem-core/src/net/upstream_grant/mcp.rs", "async fn attach_http", "descriptor", "McpHttpAdopted"),
    ("capsem-service/src/gateway_grant.rs", "async fn send_grant", "descriptor", "GatewayGrantRequest::Adopted"),
    ("capsem-service/src/proxy_worker.rs", "async fn grant(&mut self", "descriptors", "ProxyControlEvent::Adopted"),
    ("capsem-service/src/proxy_worker.rs", "impl PendingWorker", "stream", "ProxyControlEvent::Adopted"),
    ("capsem-service/src/ledger_worker.rs", "async fn connect(&mut self", "worker", "LedgerControlEvent::Adopted"),
    ("capsem-core/src/net/upstream_grant.rs", "\nasync fn attach_proxy_mcp", "descriptor", "ProxyMcpAdopted"),
    ("capsem-core/src/net/upstream_grant.rs", "\nasync fn attach_proxy_trace_hints", "descriptor", "ProxyTraceHintsAdopted"),
    ("capsem-core/src/net/upstream_grant.rs", "\nasync fn attach_proxy_traffic", "descriptor", "ProxyTrafficAdopted"),
]


def without_prose(source: str) -> str:
    # Ignore prose, including strings that pretend to contain receipts/drops.
    return re.sub(r'//[^\n]*|/\*.*?\*/|r(#+)".*?"\1|"(?:\\.|[^"\\])*"', "", source, flags=re.S)


def handoff(source: str, marker: str) -> str:
    section = without_prose(source).split(marker, 1)[1]
    if marker.startswith("impl "):
        section = section.split("async fn grant", 1)[1]
    return section.split("\n    async fn ", 1)[0].split("\nasync fn ", 1)[0]


def retains_until_receipt(source: str, marker: str, owner: str, receipt: str) -> bool:
    section = handoff(source, marker)
    release = re.search(rf"\bdrop\s*\(\s*{owner}\s*\)", section)
    adoption = section.find(receipt)
    return release is not None and 0 <= adoption < release.start()


@pytest.mark.parametrize(("path", "marker", "owner", "receipt"), HANDOFFS)
def test_original_socket_outlives_adoption(path: str, marker: str, owner: str, receipt: str) -> None:
    assert retains_until_receipt((ROOT / "crates" / path).read_text(), marker, owner, receipt), (
        f"{path}: {marker} releases {owner} before adoption.\n{ADOPTION_RATIONALE}"
    )


@pytest.mark.parametrize("owner", ["stream", "worker", "descriptors", "descriptor"])
def test_guard_rejects_early_drop_even_with_receipt_in_prose(owner: str) -> None:
    source = f'''async fn grant() {{
        // Adopted
        let message = "Adopted";
        drop ({owner});
        match receive().await? {{ Event::Adopted => Ok(()), _ => Err(()) }}
    }}'''
    assert not retains_until_receipt(source, "async fn grant", owner, "Event::Adopted")
    corrected = source.replace(f"drop ({owner});", "").replace("\n    }", f"\n        drop({owner});\n    }}")
    assert retains_until_receipt(corrected, "async fn grant", owner, "Event::Adopted")


def aggregator_receipt_contract(server: str, client: str) -> bool:
    server = re.sub(r"\s+", "", without_prose(server))
    client = re.sub(r"\s+", "", handoff(client, "async fn open_aggregator"))
    return all(token in server for token in (
        "pending_aggregator=Some(descriptor);",
        "awaiting_adoption=Some(AwaitingAdoption::Aggregator(request_id));",
        "awaiting_adoption==Some(AwaitingAdoption::Aggregator(grant_id))",
        "pending_aggregator.take().ok_or(",
    )) and "send_request(sender,&UpstreamGrantRequest::Adopted{grant_id:request_id}).await?;" in client


def test_aggregator_owner_connection_requires_a_matching_receipt() -> None:
    server = (ROOT / "crates/capsem-service/src/upstream_broker.rs").read_text()
    client = (ROOT / "crates/capsem-core/src/net/upstream_grant/mcp.rs").read_text()
    assert aggregator_receipt_contract(server, client), ADOPTION_RATIONALE
    assert not aggregator_receipt_contract(server.replace("pending_aggregator = Some(descriptor);", ""), client)
    assert not aggregator_receipt_contract(server, client.replace(".await?;", ".await;"))


def test_service_checks_cable_answer_identity_before_acknowledging() -> None:
    source = without_prose((ROOT / "crates/capsem-service/src/switches.rs").read_text())
    source = re.sub(r"\s+", "", source)
    assert "answer.bytes==seat_frame(SEAT_LINK,token)" in source, ADOPTION_RATIONALE
    assert "fd::validate_connected_stream(stream.as_fd())?;" in source, ADOPTION_RATIONALE


def python_coordinator_receipts(source: str) -> bool:
    method = next(node for node in ast.walk(ast.parse(source))
                  if isinstance(node, ast.FunctionDef) and node.name == "_serve_grants")
    for owner in ast.walk(method):
        if not isinstance(owner, ast.With) or not any(
            isinstance(item.context_expr, ast.Name) and item.context_expr.id == "granted" for item in owner.items
        ):
            continue
        if any(isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
               and node.func.attr == "close" and isinstance(node.func.value, ast.Name)
               and node.func.value.id == "granted" for node in ast.walk(owner)):
            continue
        fields = {}
        receives = False
        for node in ast.walk(owner):
            if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant):
                for target in node.targets:
                    if (isinstance(target, ast.Subscript) and isinstance(target.value, ast.Name)
                            and target.value.id == "adopted" and isinstance(target.slice, ast.Constant)):
                        fields[target.slice.value] = node.value.value
            if not isinstance(node, ast.If) or not isinstance(node.test, ast.Compare):
                continue
            check = node.test
            receives |= (
                isinstance(check.left, ast.Call) and isinstance(check.left.func, ast.Attribute)
                and check.left.func.attr == "_recv_exact" and len(check.ops) == 1
                and isinstance(check.ops[0], ast.NotEq) and len(check.comparators) == 1
                and isinstance(check.comparators[0], ast.Name) and check.comparators[0].id == "adopted"
                and any(isinstance(statement, ast.Return) for statement in node.body)
            )
        if fields == {3: 3, 12: 0} and receives:
            return True
    return False


def test_python_gateway_fixture_owns_the_socket_through_its_receipt() -> None:
    source = (ROOT / "tests/helpers/gateway.py").read_text()
    assert python_coordinator_receipts(source), ADOPTION_RATIONALE
    assert not python_coordinator_receipts(source.replace("with granted:", "with contextlib.nullcontext():"))
    assert not python_coordinator_receipts(source.replace("self._recv_exact(channel, len(adopted))", "b''"))
    assert not python_coordinator_receipts(source.replace("adopted[3] = 3", "adopted[3] = 1"))
    assert not python_coordinator_receipts(source.replace("with granted:", "with granted:\n                        granted.close()"))
