"""Internal MessagePack must have one typed, sparse protocol owner.

An untyped encoder can quietly turn a default into stored nulls or arrays, or
emit positional MessagePack whose fields cannot be omitted safely. Catch that
source shape before the expensive build, alongside the encoded-key Rust tests.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROTOCOL = ROOT / "crates/capsem-proto/src"
WIRE_TYPES = (
    "lib.rs",
    "ipc.rs",
    "handshake.rs",
    "mcp_aggregator.rs",
    "mcp_contracts.rs",
    "mcp_contracts/builtin_ledger.rs",
    "repeated.rs",
)
ENCODER_OWNERS = {
    Path("crates/capsem-proto/src/lib.rs"),
    Path("crates/capsem-proto/src/exec_stream.rs"),
    Path("crates/capsem-proto/src/mcp_aggregator.rs"),
    Path("crates/capsem-proto/src/repeated.rs"),
    Path("crates/capsem-foundation/src/ipc_channel.rs"),
    Path("crates/capsem-foundation/src/ipc_handshake.rs"),
}
TYPE_START = re.compile(r"\b(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+\w+[^;]*\{")
FIELD = re.compile(r"\b(?:pub\s+)?\w+\s*:\s*Option<")
ENCODER = re.compile(r"rmp_serde::(?:to_vec(?:_named)?|encode::write(?:_named)?|Serializer)\b")

SPARSE_MSGPACK_RATIONALE = """\
Capsem-owned MessagePack must use named, typed capsem-proto fields with
paired serde(default, skip_serializing_if) for optional/default values.
Unpaired defaults write empty/null bytes; positional or ad-hoc encoders
silently evade that contract. Keep transport framing in its existing owners.
"""


def default_field_violations(source: str) -> list[str]:
    """Return defaultable wire fields whose omission/decoding pair is incomplete."""
    violations: list[str] = []
    depth = 0
    attributes: list[str] = []
    attribute_open = False
    for number, original in enumerate(source.splitlines(), 1):
        line = original.split("//", 1)[0].strip()
        if depth == 0:
            if TYPE_START.search(line):
                depth = line.count("{") - line.count("}")
            continue
        if line.startswith("#[serde(") or attribute_open:
            attributes.append(line)
            attribute_open = not line.endswith(")]")
        elif line.startswith("#[") or not line:
            pass
        else:
            attrs = " ".join(attributes)
            missing_pair = "default" not in attrs or "skip_serializing_if" not in attrs
            if (FIELD.search(line) and missing_pair) or ("default" in attrs and "skip_serializing_if" not in attrs):
                violations.append(f"line {number}: {line}")
            attributes.clear()
        depth += line.count("{") - line.count("}")
    return violations


def encoder_violations(relative: Path, source: str) -> list[str]:
    if "/tests/" in f"/{relative.as_posix()}/" or relative.name == "tests.rs":
        return []
    if relative not in ENCODER_OWNERS:
        return [f"{relative}: MessagePack codec outside protocol/framing owner"] if "rmp_serde" in source else []
    calls = ENCODER.findall(source)
    return [f"{relative}: positional MessagePack encoder" for call in calls if call in ("rmp_serde::to_vec", "rmp_serde::encode::write")]


def test_guard_rejects_missing_omission_and_wild_encoding() -> None:
    missing = "pub struct Event {\n    pub value: Option<String>,\n}"
    assert default_field_violations(missing) == ["line 2: pub value: Option<String>,"]
    default_scalar = "pub struct Event {\n    #[serde(default)]\n    pub count: u64,\n}"
    assert default_field_violations(default_scalar) == ["line 3: pub count: u64,"]
    multiline = "pub struct Event {\n    #[serde(\n        default,\n    )]\n    pub count: u64,\n}"
    assert default_field_violations(multiline) == ["line 5: pub count: u64,"]
    assert encoder_violations(Path("crates/rogue/src/lib.rs"), "rmp_serde::to_vec_named(&event)")
    assert encoder_violations(Path("crates/rogue/src/lib.rs"), "use rmp_serde as codec;")
    assert encoder_violations(Path("crates/capsem-proto/src/lib.rs"), "rmp_serde::to_vec(&event)")


def test_capsem_messagepack_is_sparse_and_owned() -> None:
    violations = [
        f"{name}: {problem}"
        for name in WIRE_TYPES
        for problem in default_field_violations((PROTOCOL / name).read_text())
    ]
    for path in (ROOT / "crates").glob("*/src/**/*.rs"):
        relative = path.relative_to(ROOT)
        violations.extend(encoder_violations(relative, path.read_text()))
    assert not violations, SPARSE_MSGPACK_RATIONALE + "\n" + "\n".join(violations)
