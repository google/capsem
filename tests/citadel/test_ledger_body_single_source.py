"""Citadel guard: one body field per direction in the session-ledger events.

`NetEvent` and `ModelCall` used to carry a body twice: a `*_body_preview`
string that the writer wrote to a display column, and a `*_body_full` string
that the writer staged into the body archive. Every producer built both from
the same buffer, so the proxy cloned each captured body a second time on the
hot path, and every constructor in the tree had two fields to keep honest.
Nothing enforced that they agreed: a producer that filled `request_body_full`
and left `request_body_preview` at `None` shipped a row whose preview column
was empty while the archive held the body, and the reverse combination
archived a truncated excerpt as if it were the whole body.

There is one body. The event carries the bytes once, and the writer derives
the display preview at insert (`cap_preview` over a lossy UTF-8 view) beside
the row it belongs to -- which is the only place that knows what the display
column is for. The DB columns keep their names; this is about the struct.

See CLAUDE.md 'Logger DB Boundary' and skills/dev-session-debug.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
EVENTS = PROJECT_ROOT / "crates/capsem-logger/src/events.rs"

BODY_SINGLE_SOURCE_RATIONALE = """\
A session-ledger event declares one field per body direction.

Do not reintroduce a `*_body_preview` beside a `*_body_full`/`*_body` on the
same struct: the producer would build the body twice, nothing would hold the
two copies to each other, and a disagreement between them is a ledger that
reports one body in the timeline and stores a different one in the archive.
The writer derives the preview at insert from the single field.
"""

STRUCT_RE = re.compile(r"^pub struct (\w+)\s*\{", re.MULTILINE)
FIELD_RE = re.compile(r"^\s*pub (\w+)\s*:", re.MULTILINE)


def struct_fields(text: str) -> dict[str, list[str]]:
    """Every `pub struct` in the source, mapped to its public field names.

    Brace-counted rather than regex-delimited so a nested type in a field
    position cannot end the struct early.
    """
    fields: dict[str, list[str]] = {}
    for match in STRUCT_RE.finditer(text):
        depth = 0
        end = match.end() - 1
        for index in range(match.end() - 1, len(text)):
            if text[index] == "{":
                depth += 1
            elif text[index] == "}":
                depth -= 1
                if depth == 0:
                    end = index
                    break
        fields[match.group(1)] = FIELD_RE.findall(text[match.end() : end])
    return fields


def body_source_violations(path: str, text: str) -> list[str]:
    """Pure predicate over (path, text): every struct holding a body twice."""
    violations: list[str] = []
    for name, names in struct_fields(text).items():
        held = set(names)
        for field in names:
            if field.endswith("_body_preview"):
                stem = field[: -len("_body_preview")]
                for twin in (f"{stem}_body_full", f"{stem}_body"):
                    if twin in held:
                        violations.append(f"{path}: {name} declares both `{field}` and `{twin}`")
            elif field.endswith("_body_full"):
                stem = field[: -len("_body_full")]
                if f"{stem}_body" in held:
                    violations.append(f"{path}: {name} declares both `{field}` and `{stem}_body`")
    return violations


def test_the_events_source_exists() -> None:
    """A guard over a file nobody has asserts nothing."""
    assert EVENTS.is_file(), f"{EVENTS} is missing; this guard is vacuous"


def test_no_event_carries_its_body_twice() -> None:
    violations = body_source_violations(str(EVENTS.relative_to(PROJECT_ROOT)), EVENTS.read_text())
    assert not violations, BODY_SINGLE_SOURCE_RATIONALE + "\n" + "\n".join(violations)


def test_the_predicate_flags_a_struct_that_holds_a_body_twice() -> None:
    """The adversarial case: the exact shape this guard burned."""
    adversarial = """
pub struct NetEvent {
    pub domain: String,
    pub request_body_preview: Option<String>,
    pub response_body_preview: Option<String>,
    pub request_body_full: Option<String>,
    pub response_body: Option<Vec<u8>>,
}
"""
    violations = body_source_violations("adversarial.rs", adversarial)
    assert len(violations) == 2, violations


def test_the_predicate_allows_one_field_per_direction() -> None:
    """The legitimate shape: the bytes once, plus previews of other things."""
    honest = """
pub struct NetEvent {
    pub request_body: Option<Vec<u8>>,
    pub response_body: Option<Vec<u8>>,
    pub request_headers: Option<String>,
}

pub struct ModelCall {
    pub system_prompt_preview: Option<String>,
    pub request_body: Option<Vec<u8>>,
    pub response_body: Option<Vec<u8>>,
    pub text_content: Option<String>,
}
"""
    assert body_source_violations("honest.rs", honest) == []
