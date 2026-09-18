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

Three things a first cut got wrong, fixed before the routes arrive rather than
after:

- it hardcoded axum 0.8's `{event_id}`, so the 0.7 `:event_id` spelling walked
  past it. Both are matched; a downgrade is not a bypass.
- its idea of "reaches the database" was a list of `DbHandle` methods, so a
  handler that called a helper which queried was invisible. A handler that
  hands the id to anything it does not validate first is now the trigger.
- it built one function map across every crate, so two crates with a
  same-named function silently overwrote each other. The map is per file, and
  a handler resolved in more than one place is reported rather than guessed at.

Today there are no such handlers, and this guard passes by finding none. That
is deliberate. `validate_event_id` arrives with the body routes, and a guard
written afterwards is a guard written around whatever those routes happened to
do. The adversarial cases below are what prove it is not vacuous, and the
vacuity check asserts the route pattern matches a real captured route in the
tree so it cannot report zero because it stopped parsing.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import itertools
import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES = PROJECT_ROOT / "crates"

VALIDATOR = "validate_event_id("

# axum 0.8 spells a path capture `{event_id}`; 0.7 spelled it `:event_id`.
#
# `rest` stops at the next `.route(`. It used to run to the statement's
# semicolon, which in a builder chain is the end of the whole router: every
# handler registered after the `{event_id}` route was read as a handler *of*
# it. That is how a reverse proxy in another crate, which never sees an event
# id, came to be reported as a handler that reads one unvalidated.
ROUTE = re.compile(
    r"""\.route\(\s*(?P<q>["'])(?P<path>[^"']*(?:\{event_id\}|:event_id\b)[^"']*)(?P=q)\s*,"""
    r"""(?P<rest>(?:(?!\.route\s*\()[^;])*)"""
)
# Any captured segment, in either spelling: used only to prove the scanner can
# still see routes when no {event_id} one exists.
ANY_CAPTURE = re.compile(r"""\.route\(\s*["'][^"']*(?:\{\w+\}|:\w+)[^"']*["']""")
HANDLER_NAME = re.compile(r"\b(?:get|post|put|patch|delete|head|options)\s*\(\s*([\w:]+)")
FN_START = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)", re.M)

# What "reaching the database" looks like from a route handler. Plain
# substrings, searched for their earliest occurrence: the question is only
# which came first, the validation or the query.
#
# A handler that passes the id to a helper which queries is the same thing with
# an extra frame, and listing DbHandle's methods would never catch it. `USES_ID`
# is the general form -- the id reaching any call at all -- and DB_CALLS stays
# because it gives the better message when it is the direct shape.
DB_CALLS = (".query(", ".query_many(", ".query_one(", ".write(", ".ready(", ".bodies(")
USES_ID = re.compile(r"(?P<callee>\w+)\s*\([^)]*?&?\s*\bevent_id\b")

# Receivers whose method of the same name is not the ledger's. `Uri::query`
# returns the query string of the request being served; a handler that reads
# its own URI has not read the database, and reporting it would leave the
# handler no shape it could be written in.
#
# The leading dot is load-bearing. A bare `uri()` suffix also matched
# `db_uri()`, `session_uri()` and `conn_uri()` -- three names anyone could
# reach for, each of which would have exempted the `.query(` after it. What is
# left is `db.uri().query(`, where a field genuinely named `uri` returns
# something with a `query` method: inherent to matching text rather than types,
# and narrow enough to accept.
NOT_A_RECEIVER = (".uri()",)

# Callees that take the id and are not a use of it: the validator itself, and
# axum's extractor binding it.
NOT_A_USE = frozenset({"validate_event_id", "Path"})


def statements_of(function: str) -> str:
    """The function's body, without its signature.

    The signature names the id too -- `fn handle(Path(event_id): ...)` is the
    parameter list, not a handoff -- and scanning it would make every handler
    look like it used the id before its first line.
    """
    opened = function.find("{")
    return function[opened + 1 :] if opened != -1 else function


def first_db_call(body: str) -> int | None:
    """Where the body first reaches the database, skipping look-alikes."""
    earliest: list[int] = []
    for call in DB_CALLS:
        at = 0
        while (hit := body.find(call, at)) != -1:
            if not body[:hit].rstrip().endswith(NOT_A_RECEIVER):
                earliest.append(hit)
                break
            at = hit + 1
    return min(earliest, default=None)


def first_handoff(body: str) -> int | None:
    """Where the id is first given to something that has not checked it."""
    for match in USES_ID.finditer(body):
        if match.group("callee") not in NOT_A_USE:
            return match.start()
    return None

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
    out: dict[str, str] = {}
    for begin, end in itertools.pairwise([*starts, len(text)]):
        body = text[begin:end]
        name = FN_START.search(body)
        if name:
            out[name.group(1)] = body
    return out


def _handler_bodies(sources: dict[str, str]) -> dict[str, list[tuple[str, str]]]:
    """name -> [(file, body)].

    A list, not a single body: two crates may each define `handle_event_body`,
    and a flat map would have let the last one read win. An ambiguous name is
    reported rather than resolved by accident.
    """
    out: dict[str, list[tuple[str, str]]] = {}
    for path, text in sources.items():
        for name, body in _functions(text).items():
            out.setdefault(name, []).append((path, body))
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
    """Pure predicate over the tree: handlers that use the id unvalidated."""
    bodies = _handler_bodies(sources)

    violations: list[str] = []
    for path, handler in event_id_handlers(sources):
        found = bodies.get(handler, [])
        if not found:
            violations.append(f"{path}: handler `{handler}` not found; this guard cannot see it")
            continue
        if len(found) > 1:
            where = ", ".join(file for file, _ in found)
            violations.append(
                f"{path}: `{handler}` is defined in more than one place ({where}); "
                "this guard cannot tell which one the route uses"
            )
            continue
        body = statements_of(found[0][1])

        direct = first_db_call(body)
        passed_on = first_handoff(body)
        first_use = min(
            [at for at in (direct, passed_on) if at is not None],
            default=None,
        )
        if first_use is None:
            continue

        validated = body.find(VALIDATOR)
        what = "reaches the database" if first_use == direct else "hands the id on"
        if validated == -1:
            violations.append(f"{path}: `{handler}` {what} without {VALIDATOR}")
        elif validated > first_use:
            violations.append(f"{path}: `{handler}` calls {VALIDATOR} after it {what}")
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
    # Stronger: the route regex itself must match a real route with a capture.
    # "there are no {event_id} routes" and "the pattern stopped matching" look
    # identical from the outside, and only one of them is good news.
    captured = [
        match.group(0)
        for text in sources.values()
        for match in ANY_CAPTURE.finditer(text)
    ]
    assert captured, (
        "the route pattern matches no captured route anywhere in the tree; "
        "it has stopped parsing rather than found nothing to report"
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
    assert "after it reaches the database" in found[0], found


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


def test_the_predicate_reads_the_older_axum_spelling() -> None:
    """A downgrade to axum 0.7's `:event_id` is not a way around this."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/:event_id/bodies", get(handle_event_bodies))
}
'''
    handler = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    Json(db.query("SELECT 1", &[]).await).into_response()
}
'''
    found = unvalidated_handlers({"router.rs": router, "handler.rs": handler})
    assert len(found) == 1, found


def test_the_predicate_follows_the_id_into_a_helper() -> None:
    """The query one frame down is the same unvalidated read."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    handler = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    Json(load_bodies_for(&db, &event_id).await).into_response()
}
'''
    found = unvalidated_handlers({"router.rs": router, "handler.rs": handler})
    assert len(found) == 1, found
    assert "hands the id on" in found[0], found


def test_an_ambiguous_handler_name_is_reported_not_guessed() -> None:
    """Two crates, one name: the guard must not pick one and hope."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    safe = '''
async fn handle_event_bodies(event_id: String) -> Response {
    let event_id = validate_event_id(&event_id)?;
    Json(db.query("SELECT 1", &[]).await).into_response()
}
'''
    unsafe_copy = '''
async fn handle_event_bodies(event_id: String) -> Response {
    Json(db.query("SELECT 1", &[]).await).into_response()
}
'''
    found = unvalidated_handlers(
        {"router.rs": router, "a/handler.rs": safe, "b/handler.rs": unsafe_copy}
    )
    assert len(found) == 1, found
    assert "more than one place" in found[0], found


def test_a_later_route_in_the_same_chain_is_not_this_routes_handler() -> None:
    """A builder chain is many routes, not one with many handlers.

    `rest` ran to the semicolon, so every handler registered after the
    `{event_id}` route in the same chain was read as one of its handlers. The
    gateway's reverse proxy -- which never sees an event id and serves a
    hundred paths through one function -- was reported that way.
    """
    router = '''
fn routes() -> Router {
    Router::new()
        .route("/events/{event_id}/bodies", get(handle_event_bodies))
        .route("/networks/{id}/logs", get(handle_network_logs))
}
'''
    validated = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    let event_id = validate_event_id(&event_id)?;
    Json(db.query("SELECT 1", &[]).await).into_response()
}
'''
    unrelated = '''
async fn handle_network_logs(db: DbHandle) -> Response {
    Json(db.query("SELECT 1", &[]).await).into_response()
}
'''
    found = unvalidated_handlers(
        {"router.rs": router, "a.rs": validated, "b.rs": unrelated}
    )
    assert found == [], found


def test_a_uri_accessor_is_not_a_database_call() -> None:
    """`Uri::query` shares a name with `DbHandle::query` and nothing else.

    A reverse proxy registered on the body route reads its own request URI. It
    never touches the ledger and never names an event id, and there is no shape
    it could be rewritten in that would satisfy a guard counting that as a
    read.
    """
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_proxy))
}
'''
    proxy = '''
async fn handle_proxy(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let query_present = req.uri().query().is_some();
    forward(state, req, query_present).await
}
'''
    assert unvalidated_handlers({"router.rs": router, "proxy.rs": proxy}) == []


def test_a_function_named_like_a_uri_does_not_exempt_the_call_after_it() -> None:
    """The exemption is `.uri()` the method, not `uri()` the suffix.

    `db_uri()`, `session_uri()` and `conn_uri()` are names anyone could reach
    for, and a bare suffix match let each of them carry an unvalidated read
    past this guard.
    """
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    for escape in ("db_uri()", "session_uri()", "conn_uri()"):
        handler = f'''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {{
    let rows = {escape}.query("SELECT 1", &[]).await;
    validate_event_id(&event_id)?;
    Json(rows).into_response()
}}
'''
        found = unvalidated_handlers({"router.rs": router, "handler.rs": handler})
        assert len(found) == 1, (escape, found)
        assert "after it reaches the database" in found[0], (escape, found)


def test_a_real_database_call_is_still_a_database_call() -> None:
    """The narrowing above is by receiver, not by method name."""
    router = '''
fn routes() -> Router {
    Router::new().route("/events/{event_id}/bodies", get(handle_event_bodies))
}
'''
    handler = '''
async fn handle_event_bodies(Path(event_id): Path<String>, db: DbHandle) -> Response {
    let unused = req.uri().query().is_some();
    let rows = db.query("SELECT 1", &[]).await;
    validate_event_id(&event_id)?;
    Json(rows).into_response()
}
'''
    found = unvalidated_handlers({"router.rs": router, "handler.rs": handler})
    assert len(found) == 1, found
    assert "after it reaches the database" in found[0], found


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
