"""Citadel guard: nothing an upstream controls reaches a column uncapped.

Two columns in the ledger are written from bytes a remote server chose, and
both used to be bounded only by hope.

A `*_preview` column is a display excerpt. The forensic copy is the archived
body, so the preview exists to fill one screen of a UI list -- and a session
once carried 75 MB of them, averaging 28 KB each, mirrored into RAM by two
processes on top of the identical bytes stored beside them. `cap_preview`
holds them to `PREVIEW_BYTES`.

A `*_headers` column shared `MAX_FIELD_BYTES` with model text: 256 KB, for a
field that really averages about 300 bytes. That gap is not a bound, it is a
budget, and a server that wants the ledger to cost a gigabyte only has to pad
one response header and be talked to four thousand times. `cap_headers` holds
them to `HEADER_BYTES` and reports the cut into `net_events.headers_truncated`,
because a header set that stops mid-line must not read as one that ended
there.

What this guard checks is the step that is easy to skip: an INSERT that binds
the raw field instead of the capped local. The caps are one line each, which
is exactly why a new insert forgets them, and the failure is invisible --
correct rows, quietly unbounded, until an upstream notices.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WRITER = PROJECT_ROOT / "crates/capsem-logger/src/writer.rs"
WRITER_DIR = PROJECT_ROOT / "crates/capsem-logger/src/writer"

# The column families written from upstream-controlled bytes, and the helpers
# any one of which bounds them. A trailing `(` means the name is a call, not a
# column, so the helper's own name never counts as the thing it guards.
CAPPED_COLUMNS: tuple[tuple[re.Pattern[str], tuple[str, ...]], ...] = (
    # `body_preview(bytes)` derives the excerpt from the raw body and is
    # already held to PREVIEW_BYTES; `cap_preview(&opt)` bounds one that
    # arrived as a string. Either is a cap; neither being present is not.
    (re.compile(r"\b(\w*_preview)\b(?!\s*\()"), ("cap_preview(", "body_preview(")),
    (re.compile(r"\b(request_headers|response_headers)\b(?!\s*\()"), ("cap_headers(",)),
)

INSERT = re.compile(r"INSERT\s+INTO", re.IGNORECASE)

PREVIEWS_ARE_BOUNDED_RATIONALE = """\
A column written from bytes an upstream chose is capped before it is stored.

Preview columns go through cap_preview() and header columns through
cap_headers(); both are in crates/capsem-logger/src/writer.rs. An INSERT that
names one of those columns must have the matching helper in the same function,
so the value it binds is the capped local rather than the raw field.

Without it the row is correct and unbounded at once: a hostile server pads a
header or a body, and every request costs the ledger -- and the hot in-RAM
mirror that two processes hold -- whatever the server felt like sending.
"""


def _functions(text: str) -> list[tuple[str, str]]:
    """Split a Rust source into (name, body) at `fn` boundaries.

    Crude on purpose: the question is only whether the cap call and the INSERT
    that needs it sit in the same function, and a split at column-zero `fn`
    answers that without a parser.
    """
    starts = [m.start() for m in re.finditer(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+", text, re.M)]
    if not starts:
        return []
    bounds = starts + [len(text)]
    out = []
    for begin, end in zip(starts, bounds[1:]):
        body = text[begin:end]
        name = re.search(r"fn\s+(\w+)", body)
        out.append((name.group(1) if name else "?", body))
    return out


def uncapped_inserts(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): INSERTs that bind a raw capped column."""
    violations: list[str] = []
    for name, body in _functions(text):
        if not INSERT.search(body):
            continue
        for pattern, helpers in CAPPED_COLUMNS:
            columns = sorted({m.group(1) for m in pattern.finditer(body)})
            if columns and not any(helper in body for helper in helpers):
                wanted = " or ".join(helpers)
                violations.append(
                    f"{path}: fn {name} inserts {', '.join(columns)} without {wanted}"
                )
    return violations


def writer_sources() -> list[Path]:
    sources = [WRITER]
    if WRITER_DIR.is_dir():
        sources.extend(sorted(WRITER_DIR.rglob("*.rs")))
    return [
        path
        for path in sources
        if path.name != "tests.rs" and "tests" not in path.parts
    ]


def test_the_writer_and_its_caps_exist() -> None:
    """A guard over files nobody has asserts nothing."""
    assert WRITER.is_file(), f"{WRITER} is missing; this guard is vacuous"
    text = WRITER.read_text()
    for helper, limit in (
        ("fn cap_preview", "PREVIEW_BYTES"),
        ("fn body_preview", "PREVIEW_BYTES"),
        ("fn cap_headers", "HEADER_BYTES"),
    ):
        assert helper in text, f"{helper} is gone; this guard no longer describes the writer"
        assert limit in text, f"{limit} is gone; {helper} has nothing to cap to"


def test_every_insert_caps_what_an_upstream_controls() -> None:
    violations: list[str] = []
    for source in writer_sources():
        violations.extend(uncapped_inserts(str(source.relative_to(PROJECT_ROOT)), source.read_text()))

    assert not violations, PREVIEWS_ARE_BOUNDED_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_a_raw_field() -> None:
    """The adversarial case: the insert as it would be written by accident."""
    raw = """
fn insert_net_event(conn: &Connection, event: &NetEvent) -> rusqlite::Result<()> {
    execute_cached(
        conn,
        "INSERT INTO net_events (request_headers, response_headers, request_body_preview)
         VALUES (?1, ?2, ?3)",
        params![event.request_headers, event.response_headers, event.request_body_preview],
    )
}
"""
    found = uncapped_inserts("crates/capsem-logger/src/writer/traffic_rows.rs", raw)
    assert len(found) == 2, found
    assert any("cap_headers(" in line for line in found), found
    assert any("cap_preview(" in line for line in found), found


def test_a_body_derived_preview_counts_as_capped() -> None:
    """`body_preview` cuts at PREVIEW_BYTES before it ever builds a String."""
    derived = """
fn insert_net_event(conn: &Connection, event: &NetEvent) -> rusqlite::Result<()> {
    let req_body = body_preview(event.request_body.as_deref());
    let (req_headers, cut) = cap_headers(&event.request_headers);
    execute_cached(
        conn,
        "INSERT INTO net_events (request_headers, request_body_preview) VALUES (?1, ?2)",
        params![req_headers, req_body],
    )
}
"""
    assert uncapped_inserts("crates/capsem-logger/src/writer/traffic_rows.rs", derived) == []


def test_the_predicate_allows_the_capped_shape() -> None:
    """The legitimate shape: cap first, bind the local."""
    capped = """
fn insert_net_event(conn: &Connection, event: &NetEvent) -> rusqlite::Result<()> {
    let (req_headers, req_cut) = cap_headers(&event.request_headers);
    let (resp_headers, resp_cut) = cap_headers(&event.response_headers);
    let req_body = cap_preview(&event.request_body_preview);
    execute_cached(
        conn,
        "INSERT INTO net_events (request_headers, response_headers, request_body_preview)
         VALUES (?1, ?2, ?3)",
        params![req_headers, resp_headers, req_body],
    )
}
"""
    assert uncapped_inserts("crates/capsem-logger/src/writer/traffic_rows.rs", capped) == []


def test_a_function_without_an_insert_is_out_of_scope() -> None:
    """Reading a preview column is not storing one."""
    reader = """
fn preview_of(row: &Row<'_>) -> rusqlite::Result<Option<String>> {
    row.get("response_body_preview")
}
"""
    assert uncapped_inserts("crates/capsem-logger/src/writer/traffic_rows.rs", reader) == []
