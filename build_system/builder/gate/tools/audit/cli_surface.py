"""Derive the `capsem` CLI's command paths from its clap enums.

The public surface audit approves these paths; the walk lives apart from the
audit so each stays small enough to read. Rust is read as text, so a shape
the walk does not understand fails closed with `SurfaceError` instead of
shrinking the surface it reports.
"""

from __future__ import annotations

import re
from typing import Any

from capsem_builder.gate import project_root

CLI_SOURCE = project_root() / "crates" / "capsem" / "src" / "main.rs"
TYPE_PATH = r"(?:[a-z_][a-z0-9_]*::)*([A-Z][A-Za-z0-9_]*)"


class SurfaceError(RuntimeError):
    """A public surface cannot be derived or violates policy."""


def _kebab_case(name: str) -> str:
    first = re.sub(r"(.)([A-Z][a-z]+)", r"\1-\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", first).lower()


def balanced_body(source: str, opening_brace: int) -> str:
    depth = 0
    in_string = False
    escaped = False
    for index in range(opening_brace, len(source)):
        char = source[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[opening_brace + 1 : index]
    raise SurfaceError("unbalanced Rust braces while deriving public surface")


def _enum_body(source: str, enum_name: str) -> str:
    match = re.search(rf"\benum\s+{re.escape(enum_name)}\s*\{{", source)
    if not match:
        raise SurfaceError(f"missing enum {enum_name} in {CLI_SOURCE}")
    return balanced_body(source, source.index("{", match.start()))


def _struct_body(source: str, struct_name: str) -> str | None:
    match = re.search(rf"\bstruct\s+{re.escape(struct_name)}\s*\{{", source)
    return balanced_body(source, source.index("{", match.start())) if match else None


def _top_level_entries(body: str) -> list[str]:
    entries: list[str] = []
    start = 0
    depth = 0
    in_string = False
    escaped = False
    for index, char in enumerate(body):
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char in "({[":
            depth += 1
        elif char in ")}]":
            depth -= 1
        elif char == "," and depth == 0:
            entries.append(body[start:index])
            start = index + 1
    tail = body[start:].strip()
    if tail:
        entries.append(tail)
    return entries


def _command_name(entry: str, variant: str) -> str:
    explicit = re.search(r"#\[command\([^]]*\bname\s*=\s*\"([^\"]+)\"", entry, re.DOTALL)
    return explicit.group(1) if explicit else _kebab_case(variant)


def _enum_variants(source: str, enum_name: str) -> list[dict[str, Any]]:
    variants: list[dict[str, Any]] = []
    for entry in _top_level_entries(_enum_body(source, enum_name)):
        match = re.search(r"(?m)^ {4}([A-Z][A-Za-z0-9_]*)\b(.*)$", entry)
        if not match:
            continue
        variant = match.group(1)
        tail = match.group(2).strip()
        attributes = entry[: match.start()]  # policy is above a variant, never in its fields
        tuple_match = re.match(rf"\(\s*{TYPE_PATH}\s*\)", tail)
        variants.append(
            {
                "name": _command_name(attributes, variant),
                "child": tuple_match.group(1) if tuple_match else None,
                # A struct variant's own fields, where a nested subcommand
                # can be declared just as in an Args struct.
                "fields": entry[match.end(1) :] if tail.startswith("{") else None,
                "flatten": bool(re.search(r"#\[command\([^]]*\bflatten\b", attributes, re.DOTALL)),
                "subcommand": bool(re.search(r"#\[command\([^]]*\bsubcommand\b", attributes, re.DOTALL)),
            }
        )
    if not variants:
        raise SurfaceError(f"no variants derived from enum {enum_name}")
    return variants


def _nested_subcommand(fields: str) -> tuple[str, bool] | None:
    """The enum a `#[command(subcommand)]` field names, and whether it is
    optional -- an optional subcommand leaves the bare command a command too.

    Without this a command whose subcommands live in a field was one leaf, so
    `images pull` could have shipped without reaching the approved surface.
    """
    match = re.search(
        rf"#\[command\(\s*subcommand\s*\)\]\s*(?:pub(?:\([^)]*\))?\s+)?\w+\s*:\s*(Option\s*<\s*)?{TYPE_PATH}",
        fields,
    )
    return (match.group(2), bool(match.group(1))) if match else None


def cli_paths(source: str, enum_name: str, prefix: str = "") -> list[str]:
    paths: list[str] = []
    for variant in _enum_variants(source, enum_name):
        name = variant["name"]
        child = variant["child"]
        if variant["flatten"]:
            if not child:
                raise SurfaceError(f"flattened {enum_name}.{name} has no child enum")
            paths.extend(cli_paths(source, child, prefix))
        elif variant["subcommand"]:
            if not child:
                raise SurfaceError(f"subcommand {enum_name}.{name} has no child enum")
            paths.extend(cli_paths(source, child, f"{prefix}{name} "))
        else:
            fields = variant["fields"]
            if child:
                # An Args struct is one command, unless it declares
                # subcommands of its own; an enum here has no policy.
                fields = _struct_body(source, child)
                if fields is None:
                    raise SurfaceError(f"tuple variant {enum_name}.{name} lacks flatten/subcommand policy")
            nested = _nested_subcommand(fields or "")
            if nested is None or nested[1]:
                paths.append(f"{prefix}{name}")
            if nested is not None:
                paths.extend(cli_paths(source, nested[0], f"{prefix}{name} "))
    return paths


def capsem_cli_source() -> str:
    """Every module of the CLI crate, tests excluded.

    Command groups live beside the code that runs them (`network_commands.rs`
    holds `NetworkCommands`), so the enum walk starts from `Commands` in
    `main.rs` and may resolve a child enum in any sibling module. A test
    module is not the CLI, and could otherwise lend the walk a struct of the
    same name.
    """
    root = CLI_SOURCE.parent
    sources = sorted(
        path
        for path in root.rglob("*.rs")
        if "tests" not in path.relative_to(root).parts and path.name != "tests.rs"
    )
    return "\n".join(path.read_text() for path in sources)


def capsem_cli_surface() -> list[str]:
    return sorted(cli_paths(capsem_cli_source(), "Commands"))
