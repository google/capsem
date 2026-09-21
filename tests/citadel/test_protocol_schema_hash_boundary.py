"""Citadel guard: every host/guest wire declaration changes the schema hash."""

import ast
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_HASH_RATIONALE = """
The exec stream changed from raw bytes to typed frames while its declarations
were absent from capsem-proto's schema hash. Immutable profile assets then
booted with a new host but failed every exec. Every separately framed
host/guest protocol must be an explicit hash input so compatibility changes
cannot keep the old digest.
""".strip()
REQUIRED_INPUTS = {
    "lib.rs",
    "ipc.rs",
    "handshake.rs",
    "router.rs",
    "exec_stream.rs",
}


def _declared_hash_inputs(source: str) -> set[str]:
    tree = ast.parse(source)
    for node in ast.walk(tree):
        if not isinstance(node, ast.Assign):
            continue
        if not any(isinstance(target, ast.Name) and target.id == "files" for target in node.targets):
            continue
        if not isinstance(node.value, (ast.List, ast.Tuple)):
            return set()
        values: set[str] = set()
        for element in node.value.elts:
            if not isinstance(element, ast.Constant) or not isinstance(element.value, str):
                return set()
            values.add(element.value)
        return values
    return set()


def test_every_host_guest_wire_protocol_changes_the_schema_hash() -> None:
    source = (ROOT / "crates/capsem-proto/build.rs").read_text(encoding="utf-8")
    # Rust's list literal is also a valid Python list once the `let` prefix and
    # trailing semicolon are removed. Parse the value instead of accepting a
    # comment or an unused string containing the required filename.
    declaration = next(
        (line.strip() for line in source.splitlines() if line.strip().startswith("let files = ")),
        "",
    )
    python_declaration = declaration.removeprefix("let ").removesuffix(";")
    actual = _declared_hash_inputs(python_declaration)
    assert actual >= REQUIRED_INPUTS, (
        f"missing schema-hash inputs: {sorted(REQUIRED_INPUTS - actual)}\n{SCHEMA_HASH_RATIONALE}"
    )


def test_schema_hash_guard_rejects_indirect_or_nonliteral_inputs() -> None:
    assert _declared_hash_inputs('files = required + ["exec_stream.rs"]') == set()
    assert _declared_hash_inputs('files = ["lib.rs", dynamic]') == set()
