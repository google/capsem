"""Citadel guard: only the redacting telemetry hook stores HTTP headers.

Headers were cloned into `net_events` unredacted while bodies were scrubbed,
so a credential observed in `x-api-key` was stored verbatim in an allowlisted
or echoed header value (google/capsem#218, owned by #229). The telemetry hook
now redacts every observed credential from both header sets and both bodies
before `build_net_event` copies them. A `NetEvent` built anywhere else must
carry no headers; one that did would bypass that redaction.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Only build_net_event in net/mitm_proxy/telemetry_hook.rs may put HTTP header
values in a NetEvent, because the hook redacts observed credentials from them
first. Any other NetEvent sets request_headers and response_headers to None;
add headers through the hook, never beside it.
"""

#: Where header values may enter a NetEvent: the redacting hook, and the
#: logger reading back rows that were already redacted when written.
ALLOWED = {
    Path("crates/capsem-core/src/net/mitm_proxy/telemetry_hook.rs"),
    Path("crates/capsem-logger/src/reader.rs"),
}
HEADER_FIELD = re.compile(r"\b(request_headers|response_headers)\s*:\s*(?!None\b)(?!Option<)\S")
NET_EVENT = re.compile(r"\bNetEvent\s*\{")


def headers_outside_the_hook(root: Path = PROJECT_ROOT) -> list[str]:
    found: list[str] = []
    for path in sorted((root / "crates").glob("*/src/**/*.rs")):
        relative = path.relative_to(root)
        # Production sources only: fixtures and benches build rows on purpose.
        if relative in ALLOWED or any("test" in part for part in relative.parts):
            continue
        inside = False
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            code = line.split("//", 1)[0]
            if NET_EVENT.search(code) and "struct" not in code:
                inside = True
            elif inside and code.strip() in {"}", "};", "})", "}),"}:
                inside = False
            if inside and HEADER_FIELD.search(code):
                found.append(f"{relative}:{number}: {code.strip()}")
    return found


def test_only_the_redacting_hook_stores_http_headers() -> None:
    found = headers_outside_the_hook()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_guard_detects_headers_set_beside_the_hook(tmp_path: Path) -> None:
    source = tmp_path / "crates/capsem-core/src/elsewhere.rs"
    source.parent.mkdir(parents=True)
    source.write_text(
        "fn a() -> NetEvent {\n    NetEvent {\n        request_headers: None,\n"
        "        response_headers: Some(raw.clone()),\n    }\n}\n",
        encoding="utf-8",
    )
    assert [line.split(": ", 1)[0] for line in headers_outside_the_hook(tmp_path)] == [
        "crates/capsem-core/src/elsewhere.rs:4"
    ]
