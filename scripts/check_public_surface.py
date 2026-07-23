#!/usr/bin/env python3
"""Fail closed when Capsem's intentionally small public surfaces change."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
POLICY_PATH = ROOT / "config" / "public-surface.toml"
CLI_SOURCE = ROOT / "crates" / "capsem" / "src" / "main.rs"
SERVICE_SOURCE = ROOT / "crates" / "capsem-service" / "src" / "main.rs"
HTTP_METHODS = ("delete", "get", "patch", "post", "put")


class SurfaceError(RuntimeError):
    """A public surface cannot be derived or violates policy."""


def _kebab_case(name: str) -> str:
    first = re.sub(r"(.)([A-Z][a-z]+)", r"\1-\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", first).lower()


def _balanced_body(source: str, opening_brace: int) -> str:
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
    return _balanced_body(source, source.index("{", match.start()))


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
    explicit = re.search(
        r"#\[command\([^]]*\bname\s*=\s*\"([^\"]+)\"", entry, re.DOTALL
    )
    return explicit.group(1) if explicit else _kebab_case(variant)


def _enum_variants(source: str, enum_name: str) -> list[dict[str, Any]]:
    variants: list[dict[str, Any]] = []
    for entry in _top_level_entries(_enum_body(source, enum_name)):
        match = re.search(r"(?m)^ {4}([A-Z][A-Za-z0-9_]*)\b(.*)$", entry)
        if not match:
            continue
        variant = match.group(1)
        tail = match.group(2).strip()
        tuple_type = None
        tuple_match = re.match(r"\(\s*([A-Z][A-Za-z0-9_]*)\s*\)", tail)
        if tuple_match:
            tuple_type = tuple_match.group(1)
        variants.append(
            {
                "name": _command_name(entry, variant),
                "child": tuple_type,
                "flatten": bool(
                    re.search(r"#\[command\([^]]*\bflatten\b", entry, re.DOTALL)
                ),
                "subcommand": bool(
                    re.search(r"#\[command\([^]]*\bsubcommand\b", entry, re.DOTALL)
                ),
            }
        )
    if not variants:
        raise SurfaceError(f"no variants derived from enum {enum_name}")
    return variants


def _cli_paths(source: str, enum_name: str, prefix: str = "") -> list[str]:
    paths: list[str] = []
    for variant in _enum_variants(source, enum_name):
        name = variant["name"]
        child = variant["child"]
        if variant["flatten"]:
            if not child:
                raise SurfaceError(f"flattened {enum_name}.{name} has no child enum")
            paths.extend(_cli_paths(source, child, prefix))
        elif variant["subcommand"]:
            if not child:
                raise SurfaceError(f"subcommand {enum_name}.{name} has no child enum")
            paths.extend(_cli_paths(source, child, f"{prefix}{name} "))
        else:
            if child:
                raise SurfaceError(
                    f"tuple variant {enum_name}.{name} lacks flatten/subcommand policy"
                )
            paths.append(f"{prefix}{name}")
    return paths


def capsem_cli_surface() -> list[str]:
    return sorted(_cli_paths(CLI_SOURCE.read_text(), "Commands"))


def just_surface() -> list[str]:
    completed = subprocess.run(
        ["just", "--dump", "--dump-format", "json"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    recipes = json.loads(completed.stdout)["recipes"]
    return sorted(
        name for name, recipe in recipes.items() if not recipe.get("private", False)
    )


def _function_body(source: str, function_name: str) -> str:
    match = re.search(
        rf"\bfn\s+{re.escape(function_name)}\s*\([^)]*\)[^{{]*\{{", source
    )
    if not match:
        raise SurfaceError(f"missing function {function_name} in {SERVICE_SOURCE}")
    return _balanced_body(source, source.index("{", match.start()))


def _route_calls(router_body: str) -> list[str]:
    calls: list[str] = []
    cursor = 0
    marker = ".route("
    while True:
        start = router_body.find(marker, cursor)
        if start < 0:
            return calls
        opening = start + len(marker) - 1
        depth = 0
        in_string = False
        escaped = False
        for index in range(opening, len(router_body)):
            char = router_body[index]
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
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
                if depth == 0:
                    calls.append(router_body[opening + 1 : index])
                    cursor = index + 1
                    break
        else:
            raise SurfaceError("unbalanced .route(...) call")


def http_surface() -> list[str]:
    source = SERVICE_SOURCE.read_text()
    router = _function_body(source, "build_service_router")
    surface: set[str] = set()
    for call in _route_calls(router):
        path_match = re.match(r'\s*"([^"]+)"\s*,', call, re.DOTALL)
        if not path_match:
            raise SurfaceError(f"route does not begin with a literal path: {call[:80]!r}")
        path = path_match.group(1)
        handler = call[path_match.end() :]
        methods = {
            match.group(1).upper()
            for match in re.finditer(
                rf"(?:\b|\.)({'|'.join(HTTP_METHODS)})\s*\(", handler
            )
        }
        if not methods:
            raise SurfaceError(f"no HTTP method derived for route {path}")
        surface.update(f"{method} {path}" for method in methods)
    return sorted(surface)


def current_surfaces() -> dict[str, list[str]]:
    return {
        "just": just_surface(),
        "capsem_cli": capsem_cli_surface(),
        "http": http_surface(),
    }


def check_policy(policy_path: Path = POLICY_PATH) -> None:
    policy = tomllib.loads(policy_path.read_text())
    current = current_surfaces()
    failures: list[str] = []
    for surface_name, actual in current.items():
        section = policy.get(surface_name)
        if not isinstance(section, dict):
            failures.append(f"{surface_name}: missing policy section")
            continue
        expected = sorted(section.get("approved", []))
        expected_count = section.get("count")
        if expected_count != len(expected):
            failures.append(
                f"{surface_name}: policy count={expected_count} but allowlist has "
                f"{len(expected)} entries"
            )
        added = sorted(set(actual) - set(expected))
        removed = sorted(set(expected) - set(actual))
        if len(actual) != expected_count or added or removed:
            failures.append(
                f"{surface_name}: approved={expected_count}, actual={len(actual)}, "
                f"unapproved={added}, missing={removed}"
            )
    if failures:
        raise SurfaceError(
            "Public surface changed without approval. Review the API intentionally, "
            "then update config/public-surface.toml in the same change:\n- "
            + "\n- ".join(failures)
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--dump-current",
        action="store_true",
        help="print the derived surfaces without approving them",
    )
    args = parser.parse_args()
    try:
        if args.dump_current:
            print(json.dumps(current_surfaces(), indent=2))
        else:
            check_policy()
            surfaces = current_surfaces()
            print(
                "public surface approved: "
                + ", ".join(f"{name}={len(values)}" for name, values in surfaces.items())
            )
    except (OSError, subprocess.CalledProcessError, SurfaceError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
