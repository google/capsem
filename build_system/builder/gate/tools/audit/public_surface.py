"""Fail closed when Capsem's intentionally small public surfaces change."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

from capsem_builder.gate import project_root
from capsem_builder.gate.tools.audit.cli_surface import (
    SurfaceError,
    balanced_body,
    capsem_cli_surface,
)

ROOT = project_root()
POLICY_PATH = ROOT / "config" / "public-surface.toml"
SERVICE_SOURCE = ROOT / "crates" / "capsem-service" / "src" / "router_runtime.rs"
HTTP_METHODS = ("delete", "get", "patch", "post", "put")


def just_surface() -> list[str]:
    completed = subprocess.run(
        ["just", "--dump", "--dump-format", "json"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    recipes = json.loads(completed.stdout)["recipes"]
    return sorted(name for name, recipe in recipes.items() if not recipe.get("private", False))


def _function_body(source: str, function_name: str) -> str:
    match = re.search(rf"\bfn\s+{re.escape(function_name)}\s*\([^)]*\)[^{{]*\{{", source)
    if not match:
        raise SurfaceError(f"missing function {function_name} in {SERVICE_SOURCE}")
    return balanced_body(source, source.index("{", match.start()))


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
            for match in re.finditer(rf"(?:\b|\.)({'|'.join(HTTP_METHODS)})\s*\(", handler)
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
            "then update config/public-surface.toml in the same change:\n- " + "\n- ".join(failures)
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
