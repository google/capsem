"""Citadel guard: an `{event_id}` from a URL is validated before it is used.

An event id is twelve lowercase hex characters, and the ledger's CHECK
constraints say so on the way in. On the way out, through a route path, it is
whatever the caller typed. Every such handler reaches a database with it, and
the first thing that touches an untrusted string decides what "twelve hex
characters" is going to mean for the rest of the request -- so it has to be
one place, not each handler's own idea of it.

Validation before the DB call, not around it: a handler that queries first and
checks afterwards has already spent the query, and a handler that checks the
id in three of its four branches is the shape this guard exists to notice.

Today there are no such handlers, and this guard passes by finding none. That
is deliberate. `validate_event_id` arrives with the body routes, and a guard
written afterwards is a guard written around whatever those routes happened to
do. The adversarial case below is what proves it is not vacuous: it flags a
handler of exactly the shape that is coming.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES = PROJECT_ROOT / "crates"

VALIDATOR = "validate_event_id("

# axum 0.8 spells a path capture `{event_id}`.
ROUTE = re.compile(r"""\.route\(\s*(?P<q>["'])(?P<path>[^"']*\{event_id\}[^"']*)(?P=q)\s*,(?P<rest>[^;]*)""")
HANDLER_NAME = re.compile(r"\b(?:get|post|put|patch|delete|head|options)\s*\(\s*([\w:]+)")
FN_START = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)", re.M)

# What "reaching the database" looks like from a route handler. Plain
# substrings, searched for their earliest occurrence: the question is only
# which came first, the validation or the query.
DB_CALLS = (".query(", ".query_many(", ".query_one(", ".write(", ".ready(", ".bodies(")

EVENT_ID_VALIDATED_RATIONALE = """\
A handler whose route path carries {event_id} must call validate_event_id()
before it touches the database.

The id in a URL is an untrusted string; the twelve-hex-character rule that the
ledger enforces on the way in is not enforced on the way out. One validator,
called first, is what keeps every route agreeing on what an event id is -- and
what keeps a handler from spending a query on a value it was going to reject.

Validate, then query. Not query, then validate.
"""


def rust_sources() -> list[Path]:
    return sorted(p for p in CRATES.rglob("*.rs") if "tests" not in p.parts and p.name != "tests.rs")


def _functions(text: str) -> dict[str, str]:
    """name -> body, split at column-zero `fn` boundaries."""
    starts = [m.start() for m in FN_START.finditer(text)]
    if not starts:
        return {}
    bounds = starts + [len(text)]
    out: dict[str, str] = {}
    for begin, end in zip(starts, bounds[1:]):
        body = text[begin:end]
        name = FN_START.search(body)
        if name:
            out[name.group(1)] = body
    return out


def event_id_handlers(sources: dict[str, str]) -> list[tuple[str, str]]:
    """(route path, handler name) for every route that captures {event_id}."""
    found: list[tuple[str, str]] = []
    for text in sources.values():
        for route in ROUTE.finditer(text):
            for handler in HANDLER_NAME.finditer(route.group("rest")):
                found.append((route.group("path"), handler.group(1).rsplit("::", 1)[-1]))
    return sorted(set(found))


def unvalidated_handlers(sources: dict[str, str]) -> list[str]:
    """Pure predicate over the tree: handlers that reach the DB unvalidated."""
    bodies: dict[str, str] = {}
    for text in sources.values():
        bodies.update(_functions(text))

    violations: list[str] = []
    for path, handler in event_id_handlers(sources):
        body = bodies.get(handler)
        if body is None:
            violations.append(f"{path}: handler `{handler}` not found; this guard cannot see it")
            continue
        offsets = [body.find(call) for call in DB_CALLS]
        db = min((at for at in offsets if at != -1), default=None)
        if db is None:
            continue
        validated = body.find(VALIDATOR)
        if validated == -1:
            violations.append(f"{path}: `{handler}` reaches the database without {VALIDATOR}")
        elif validated > db:
            violations.append(f"{path}: `{handler}` calls {VALIDATOR} after its first database call")
    return violations


def tree_sources() -> dict[str, str]:
    return {str(p.relative_to(PROJECT_ROOT)): p.read_text() for p in rust_sources()}


def test_the_guard_reads_the_real_tree() -> None:
    """Vacuity: the crates exist and the route pattern matches real routes.

    There are no `{event_id}` routes yet, so what is asserted here is that the
    scanner can see routes at all -- otherwise it would report zero for the
    wrong reason on the day the first one lands.
    """
    sources = tree_sources()
    assert sources, "no Rust sources found; this guard is vacuous"
    assert any(".route(" in text for text in sources.values()), (
        "no axum routes found in the tree; this guard's scanner is broken, not satisfied"
    )


def test_every_event_id_handler_validates_before_it_reads() -> None:
    violations = unvalidated_handlers(tree_sources())
    assert not violations, EVENT_ID_VALIDATED_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_an_unvalidated_handler() -> None:
    """The adversarial case: the route that is coming, written the easy way."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    handler = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    let rows = db.query("SELECT * FROM event_body_blobs WHERE event_id = ?1", &[&event_id]).await;
    Json(rows).into_response()
}
'''
    found = unvalidated_handlers({"router.rs": router, "handler.rs": handler})
    assert len(found) == 1, found
    assert "handle_event_bodies" in found[0] and VALIDATOR in found[0], found


def test_the_predicate_flags_validation_that_comes_too_late() -> None:
    """Checking afterwards has already spent the query."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    handler = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    let rows = db.query("SELECT 1", &[]).await;
    validate_event_id(&event_id)?;
    Json(rows).into_response()
}
'''
    found = unvalidated_handlers({"router.rs": router, "handler.rs": handler})
    assert len(found) == 1, found
    assert "after its first database call" in found[0], found


def test_the_predicate_allows_the_validated_shape() -> None:
    """Validate, then query."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    handler = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    let event_id = validate_event_id(&event_id)?;
    let rows = db.query("SELECT 1", &[]).await;
    Json(rows).into_response()
}
'''
    assert unvalidated_handlers({"router.rs": router, "handler.rs": handler}) == []


def test_a_route_without_an_event_id_is_out_of_scope() -> None:
    router = '''
fn routes() -> Router {
    Router::new().route("/networks/{id}/logs", get(handle_network_logs))
}
'''
    handler = '''
async fn handle_network_logs(db: DbHandle) -> Response {
    Json(db.query("SELECT 1", &[]).await).into_response()
}
'''
    assert unvalidated_handlers({"router.rs": router, "handler.rs": handler}) == []
