"""Fail-closed inventory of the repository's explicit Axum registrations."""
from __future__ import annotations

import re
from pathlib import Path

METHODS = {"get", "post", "put", "patch", "delete", "head", "options", "trace"}


def _methods(expression: str) -> set[str]:
    """Only the outer MethodRouter chain registers HTTP methods."""
    found = set()
    cursor = 0
    while cursor < len(expression):
        rest = expression[cursor:].lstrip().removeprefix(".").lstrip()
        match = re.match(r"([a-z_]+)\s*\(", rest)
        assert match, f"unknown MethodRouter shape: {rest}"
        name = match[1]
        assert name in METHODS | {"layer", "route_layer"}, f"unknown Axum method: {name}"
        if name in METHODS:
            found.add(name)
        offset = len(expression) - len(rest)
        cursor = offset + match.end()
        depth = 1
        quoted = False
        escaped = False
        while depth:
            assert cursor < len(expression), "unterminated MethodRouter call"
            char = expression[cursor]
            if quoted:
                if char == '"' and not escaped:
                    quoted = False
                escaped = char == "\\" and not escaped
            elif char == '"':
                quoted = True
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            cursor += 1
        if not expression[cursor:].strip().strip(","):
            break
    assert found, "route has no explicit HTTP method"
    return found


def route_operations(body: str) -> set[str]:
    found = set()
    cursor = 0
    while (call := body.find(".route(", cursor)) != -1:
        start = call + len(".route(")
        while body[start].isspace():
            start += 1
        assert body[start] == '"', "route path must be an explicit ordinary literal"
        end = body.index('"', start + 1)
        path = body[start + 1:end]
        assert "\\" not in path, "escaped route path is not supported"
        cursor = end + 1
        depth = 1
        quoted = False
        escaped = False
        while depth:
            assert cursor < len(body), "unterminated route registration"
            char = body[cursor]
            if quoted:
                if not escaped and char == '"':
                    quoted = False
                escaped = char == "\\" and not escaped
            elif char == '"':
                quoted = True
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            cursor += 1
        expression = body[end + 1:cursor - 1].lstrip().removeprefix(",").strip()
        for method in _methods(expression):
            operation = f"{method.upper()} {path}"
            assert operation not in found, f"duplicate route operation: {operation}"
            found.add(operation)
    return found


def function_operations(source: Path, function: str) -> set[str]:
    text = source.read_text(encoding="utf-8")
    start = text.index(function)
    end = text.find("\nfn ", start + len(function))
    result = route_operations(text[start:end if end != -1 else len(text)])
    assert result, f"no route operations found in {source}:{function}"
    return result
