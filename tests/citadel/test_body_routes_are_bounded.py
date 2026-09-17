"""Citadel guard: a handler that returns an archived body bounds it first.

A body in the archive is whatever an upstream sent, up to
`MAX_BODY_BLOB_BYTES`. A route that returns one is therefore a route whose
response size is chosen by a remote server, not by Capsem -- and the caller on
the other end is a UI that asked for a table row. Ten megabytes arrive at a
webview that wanted a preview, held in the service's memory on the way, once
per concurrent reader.

`bounded_body_response(` is the single place that decides how much of a body
a route may hand back and what it says when it hands back less. One place,
because the limit is only meaningful if every route has the same one: a caller
cannot tell a truncated body from a short one unless the answer is shaped the
same way every time.

Today no handler returns `capsem_api::bodies::*`, and this guard passes by
finding none. That is deliberate -- the helper and the routes land together,
and a guard written afterwards is a guard written around whatever they did.
The adversarial case below is what proves it is not vacuous.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES = PROJECT_ROOT / "crates"

BOUNDER = "bounded_body_response("

# The body API's return types, however the handler spells the path to them.
BODY_TYPE = re.compile(r"\b(?:capsem_api::)?bodies::(\w+)")
FN_SIGNATURE = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(?P<name>\w+)\s*(?P<sig>\([^{]*)->(?P<ret>[^{]*)",
    re.M | re.S,
)
FN_START = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+", re.M)

BODY_ROUTES_ARE_BOUNDED_RATIONALE = """\
A handler returning capsem_api::bodies::* must build its response through
bounded_body_response().

An archived body is as large as the upstream that sent it, up to
MAX_BODY_BLOB_BYTES. Returning one straight out of the archive makes the
response size a remote server's choice, held in the service's memory, once per
concurrent reader, for a UI that asked to fill a table row.

One bounding helper, not one per route: a caller can only tell a truncated
body from a short one if every route says so the same way.
"""


def rust_sources() -> list[Path]:
    return sorted(p for p in CRATES.rglob("*.rs") if "tests" not in p.parts and p.name != "tests.rs")


def _functions(text: str) -> list[tuple[str, str, str]]:
    """(name, return type, body) for each function, split at `fn` boundaries."""
    starts = [m.start() for m in FN_START.finditer(text)]
    if not starts:
        return []
    bounds = starts + [len(text)]
    out: list[tuple[str, str, str]] = []
    for begin, end in zip(starts, bounds[1:]):
        chunk = text[begin:end]
        signature = FN_SIGNATURE.search(chunk)
        if not signature:
            continue
        out.append((signature.group("name"), signature.group("ret"), chunk))
    return out


def unbounded_body_handlers(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): body returns that skip the bounder."""
    violations: list[str] = []
    for name, ret, body in _functions(text):
        returned = BODY_TYPE.search(ret)
        if not returned:
            continue
        if BOUNDER not in body:
            violations.append(
                f"{path}: fn {name} returns bodies::{returned.group(1)} without {BOUNDER}"
            )
    return violations


def tree_sources() -> dict[str, str]:
    return {str(p.relative_to(PROJECT_ROOT)): p.read_text() for p in rust_sources()}


def test_the_guard_reads_the_real_tree() -> None:
    """Vacuity: the scanner can see functions and their return types.

    There are no body routes yet, so what is asserted is that the parser works
    at all -- otherwise it would report zero for the wrong reason on the day
    the first route lands.
    """
    sources = tree_sources()
    assert sources, "no Rust sources found; this guard is vacuous"
    parsed = sum(len(_functions(text)) for text in sources.values())
    assert parsed > 100, f"the function parser found only {parsed} functions; it is broken"


def test_every_body_route_is_bounded() -> None:
    violations: list[str] = []
    for path, text in tree_sources().items():
        violations.extend(unbounded_body_handlers(path, text))

    assert not violations, BODY_ROUTES_ARE_BOUNDED_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_an_unbounded_body_route() -> None:
    """The adversarial case: the route that is coming, written the easy way."""
    handler = '''
async fn handle_event_body(db: DbHandle, id: EventId) -> Result<bodies::BodyResponse> {
    let bytes = db.bodies().read(id).await?;
    Ok(bodies::BodyResponse { bytes })
}
'''
    found = unbounded_body_handlers("crates/capsem-service/src/body_routes.rs", handler)
    assert len(found) == 1, found
    assert "handle_event_body" in found[0] and BOUNDER in found[0], found


def test_the_predicate_sees_a_fully_qualified_return_type() -> None:
    handler = '''
async fn handle_event_body(db: DbHandle) -> Result<capsem_api::bodies::BodyPage> {
    Ok(capsem_api::bodies::BodyPage::default())
}
'''
    assert len(unbounded_body_handlers("crates/capsem-service/src/body_routes.rs", handler)) == 1


def test_the_predicate_allows_the_bounded_shape() -> None:
    """The legitimate shape: the archive read goes through the bounder."""
    handler = '''
async fn handle_event_body(db: DbHandle, id: EventId) -> Result<bodies::BodyResponse> {
    let bytes = db.bodies().read(id).await?;
    Ok(bounded_body_response(bytes))
}
'''
    assert unbounded_body_handlers("crates/capsem-service/src/body_routes.rs", handler) == []


def test_a_handler_that_returns_no_body_is_out_of_scope() -> None:
    handler = '''
async fn handle_event_summary(db: DbHandle) -> Result<Json<Summary>> {
    Ok(Json(db.query("SELECT 1", &[]).await?))
}
'''
    assert unbounded_body_handlers("crates/capsem-service/src/ledger_routes.rs", handler) == []
