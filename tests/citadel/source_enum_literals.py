"""Remove typed wire enum values before looking for repository path references."""

from __future__ import annotations

import ast
import json
import re
from typing import Any


def contract_enums(sources: dict[str, str]) -> frozenset[str]:
    names = set()
    for path, text in sources.items():
        if not path.endswith(".json"):
            continue
        try:
            document = json.loads(text)
        except json.JSONDecodeError:
            continue
        if isinstance(document, dict) and "openapi" in document:
            schemas = document.get("components", {}).get("schemas", {})
            names.update(name for name, schema in schemas.items() if "enum" in schema)
    return frozenset(names)


def reference_source(path: str, text: str, enums: frozenset[str] = frozenset()) -> str:
    if path.endswith(".json"):
        try:
            document = json.loads(text)
        except json.JSONDecodeError:
            return text
        if isinstance(document, dict) and "openapi" in document:
            def without_enums(value: Any) -> Any:
                if isinstance(value, dict):
                    return {key: without_enums(child) for key, child in value.items() if key != "enum"}
                if isinstance(value, list):
                    return [without_enums(child) for child in value]
                return value
            return json.dumps(without_enums(document), indent=2)
    if path.endswith(".py"):
        try:
            tree = ast.parse(text)
        except SyntaxError:
            return text
        enum_types = {alias.asname or alias.name for node in tree.body
                      if isinstance(node, ast.ImportFrom) and node.module == "enum"
                      for alias in node.names if alias.name in {"Enum", "StrEnum"}}
        lines = text.splitlines(keepends=True)
        values = []
        for node in ast.walk(tree):
            if not isinstance(node, ast.ClassDef) or node.name not in enums or not any(
                isinstance(base, ast.Name) and base.id in enum_types for base in node.bases
            ):
                continue
            for member in node.body:
                if not isinstance(member, (ast.Assign, ast.AnnAssign)):
                    continue
                value = member.value
                if isinstance(value, ast.Constant) and isinstance(value.value, str) and value.lineno == value.end_lineno:
                    values.append(value)
        for value in sorted(values, key=lambda value: (value.lineno, value.col_offset), reverse=True):
            line = lines[value.lineno - 1].encode()
            lines[value.lineno - 1] = (line[:value.col_offset] + b"None" + line[value.end_col_offset:]).decode()
        return "".join(lines)
    if path.endswith(".ts"):
        # The TypeScript contract emits explicit string enums, one member per
        # line. Keep all surrounding imports, comments and path assignments.
        enum = re.compile(r"(?m)^(?:export )?enum (?P<name>\w+) \{\n(?:[ \t]+\w+ = [^\n]+\n)+\}")
        return enum.sub(lambda match: re.sub(r'(?m)^([ \t]+\w+ = )"(?:[^"\\]|\\.)*"', r'\1null', match[0])
                        if match["name"] in enums else match[0], text)
    return text
