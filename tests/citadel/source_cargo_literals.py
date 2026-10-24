"""Mask only Cargo protocol enums/command positions, never adjoining paths.

Captured original protocol files stay unchanged. Recognition is structural,
not a filename exemption; malformed or unrelated documents remain visible.
"""
from __future__ import annotations

import ast
import copy
import json
import re
from enum import StrEnum


class CargoProtocolToken(StrEnum):
    BENCH = "bench"


def _absolute(value) -> bool:
    return isinstance(value, str) and value.startswith("/")


def _target(value) -> bool:
    return isinstance(value, dict) and (
        isinstance(value.get("name"), str) and bool(value["name"])
        and isinstance(value.get("kind"), list) and all(isinstance(row, str) for row in value["kind"])
        and isinstance(value.get("crate_types"), list) and all(isinstance(row, str) for row in value["crate_types"])
        and _absolute(value.get("src_path"))
        and isinstance(value.get("edition"), str)
        and all(type(value.get(key)) is bool for key in ("doc", "doctest", "test"))
    )


def _profile(value) -> bool:
    return isinstance(value, dict) and (
        set(value) == {"opt_level", "debuginfo", "debug_assertions", "overflow_checks", "test"}
        and isinstance(value["opt_level"], str) and type(value["debuginfo"]) is int
        and all(type(value[key]) is bool for key in ("debug_assertions", "overflow_checks", "test"))
    )


def _mask_target(value: dict) -> None:
    if value["kind"] == [CargoProtocolToken.BENCH] and value["crate_types"] == ["bin"]:
        value["kind"] = [None]


def _mask(document) -> bool:
    if not isinstance(document, dict):
        return False
    if document.get("reason") == "compiler-artifact" and (
        isinstance(document.get("package_id"), str)
        and _absolute(document.get("manifest_path"))
        and _target(document.get("target"))
        and _profile(document.get("profile"))
        and isinstance(document.get("filenames"), list) and all(_absolute(row) for row in document["filenames"])
        and (document.get("executable") is None or _absolute(document["executable"]))
        and type(document.get("fresh")) is bool
    ):
        _mask_target(document["target"])
        return True
    if type(document.get("version")) is int and document["version"] == 1 and (
        isinstance(document.get("packages"), list)
        and isinstance(document.get("workspace_members"), list)
        and _absolute(document.get("target_directory"))
        and _absolute(document.get("workspace_root"))
        and all(isinstance(row, dict) and isinstance(row.get("id"), str)
                and isinstance(row.get("name"), str) and _absolute(row.get("manifest_path"))
                and isinstance(row.get("targets"), list) and all(_target(target) for target in row["targets"])
                for row in document["packages"])
    ):
        for package in document["packages"]:
            for target in package["targets"]:
                _mask_target(target)
        return True
    toolchain = re.fullmatch(r"cargo (\d+\.\d+\.\d+) \([^\n]+\)", str(document.get("toolchain", "")))
    args = document.get("args")
    if toolchain and isinstance(args, list) and all(isinstance(arg, str) for arg in args) and (
        document.get("executed") is False and type(document.get("exit_code")) is int
        and document["exit_code"] == 0 and _absolute(document.get("source_root"))
        and _absolute(document.get("target_directory"))
        and isinstance(document.get("fixture_manifest"), str)
        and isinstance(document.get("fixture_source"), str)
        and args and args[0] == "cargo" and "--no-run" in args and "--message-format=json" in args
    ):
        index = 2 if len(args) > 1 and args[1] == "+" + toolchain[1] else 1
        if len(args) > index and args[index] == CargoProtocolToken.BENCH:
            args[index] = None
            return True
    return False


def protocol_source(text: str, *, json_lines: bool) -> str | None:
    try:
        rows = [json.loads(line) for line in text.splitlines()] if json_lines else [json.loads(text)]
    except json.JSONDecodeError:
        return None
    original = copy.deepcopy(rows)
    for row in rows:
        _mask(row)
    if rows == original:
        return None
    # Other fields, including arbitrary extra paths and traversals, still scan.
    return "\n".join(json.dumps(row, indent=2) for row in rows)


def target_kind_literals(tree: ast.AST, path: str) -> list[ast.Constant]:
    found = []
    enum_bases = {alias.asname or alias.name for node in ast.walk(tree)
                  if isinstance(node, ast.ImportFrom) and node.module == "enum"
                  for alias in node.names if alias.name == "StrEnum"}
    declarations = [node for node in ast.walk(tree)
                    if isinstance(node, ast.ClassDef) and node.name == "CargoProtocolToken"]
    parents = {child: node for node in ast.walk(tree) for child in ast.iter_child_nodes(node)}
    uses = [node for node in ast.walk(tree) if isinstance(node, ast.Attribute)
            and isinstance(node.value, ast.Name) and node.value.id == "CargoProtocolToken"
            and node.attr == "BENCH"]
    comparisons_only = all(isinstance(parents.get(node), ast.Compare) or (
        isinstance(parents.get(node), ast.List)
        and isinstance(parents.get(parents[node]), ast.Compare)) for node in uses)
    for comprehension in ast.walk(tree):
        # One exact typed protocol token; fake enums and path-valued members
        # remain scanned. This is not a file-name exemption for this module.
        if (path == "tests/citadel/source_cargo_literals.py" and len(declarations) == 1
            and comparisons_only and isinstance(comprehension, ast.ClassDef) and comprehension.name == "CargoProtocolToken"
            and len(comprehension.bases) == 1 and isinstance(comprehension.bases[0], ast.Name)
            and comprehension.bases[0].id in enum_bases and len(comprehension.body) == 1):
            member = comprehension.body[0]
            if (isinstance(member, ast.Assign) and len(member.targets) == 1
                and isinstance(member.targets[0], ast.Name) and member.targets[0].id == "BENCH"
                and isinstance(member.value, ast.Constant) and member.value.value == CargoProtocolToken.BENCH):
                found.append(member.value)
        if not isinstance(comprehension, ast.ListComp) or len(comprehension.generators) != 1:
            continue
        generator = comprehension.generators[0]
        if not (isinstance(generator.target, ast.Name) and isinstance(generator.iter, ast.Subscript)
                and isinstance(generator.iter.slice, ast.Constant) and generator.iter.slice.value == "targets"):
            continue
        for condition in generator.ifs:
            for comparison in ast.walk(condition):
                if not isinstance(comparison, ast.Compare) or len(comparison.ops) != 1 or not isinstance(comparison.ops[0], ast.Eq):
                    continue
                left, right = comparison.left, comparison.comparators[0]
                if (isinstance(left, ast.Subscript) and isinstance(left.value, ast.Name)
                    and left.value.id == generator.target.id and isinstance(left.slice, ast.Constant)
                    and left.slice.value == "kind" and isinstance(right, ast.List) and len(right.elts) == 1
                    and isinstance(right.elts[0], ast.Constant) and right.elts[0].value == CargoProtocolToken.BENCH):
                    found.append(right.elts[0])
    return found
