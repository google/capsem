#!/usr/bin/env python3
"""Fail quickly on parser-level defects in checked-in source and automation."""

from __future__ import annotations

import ast
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any, ClassVar

import yaml

ROOT = Path(__file__).resolve().parents[1]
SUPPORTED_SUFFIXES = {".json", ".py", ".sh", ".toml", ".yaml", ".yml"}


class StrictYamlLoader(yaml.SafeLoader):
    """YAML 1.2-ish safe loader that rejects duplicate mapping keys."""

    yaml_implicit_resolvers: ClassVar[dict] = {
        key: list(resolvers)
        for key, resolvers in yaml.SafeLoader.yaml_implicit_resolvers.items()
    }


# PyYAML defaults to YAML 1.1 and treats GitHub's top-level `on` key as a
# boolean. Retain only the YAML 1.2 true/false spellings.
for first_character, resolvers in list(StrictYamlLoader.yaml_implicit_resolvers.items()):
    StrictYamlLoader.yaml_implicit_resolvers[first_character] = [
        (tag, regexp)
        for tag, regexp in resolvers
        if tag != "tag:yaml.org,2002:bool"
    ]
StrictYamlLoader.add_implicit_resolver(
    "tag:yaml.org,2002:bool",
    re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"),
    list("tTfF"),
)


def _construct_unique_mapping(
    loader: StrictYamlLoader,
    node: yaml.MappingNode,
    deep: bool = False,
) -> dict[Any, Any]:
    result: dict[Any, Any] = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in result:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                f"duplicate key {key!r}",
                key_node.start_mark,
            )
        result[key] = loader.construct_object(value_node, deep=deep)
    return result


StrictYamlLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_unique_mapping,
)


def _tracked_sources() -> list[Path]:
    result = subprocess.run(
        [
            "git",
            "ls-files",
            "*.json",
            "*.py",
            "*.sh",
            "*.toml",
            "*.yaml",
            "*.yml",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return [ROOT / path for path in result.stdout.splitlines() if path]


def _check_yaml(path: Path) -> None:
    documents = list(yaml.load_all(path.read_text(encoding="utf-8"), StrictYamlLoader))
    if path.is_relative_to(ROOT / ".github" / "workflows"):
        if len(documents) != 1 or not isinstance(documents[0], dict):
            raise ValueError("GitHub workflow must contain exactly one mapping document")
        workflow = documents[0]
        if "on" not in workflow:
            raise ValueError("GitHub workflow is missing top-level 'on'")
        if not isinstance(workflow.get("jobs"), dict) or not workflow["jobs"]:
            raise ValueError("GitHub workflow must define a non-empty jobs mapping")
    elif path.is_relative_to(ROOT / ".github" / "actions"):
        if len(documents) != 1 or not isinstance(documents[0], dict):
            raise ValueError("GitHub action must contain exactly one mapping document")
        if not isinstance(documents[0].get("runs"), dict):
            raise ValueError("GitHub action must define a runs mapping")


def _check(path: Path) -> str:
    suffix = path.suffix
    source = path.read_text(encoding="utf-8")
    if suffix == ".py":
        ast.parse(source, filename=str(path))
        return "Python"
    if suffix == ".sh":
        subprocess.run(
            ["bash", "-n", str(path)],
            check=True,
            capture_output=True,
            text=True,
        )
        return "shell"
    if suffix == ".json":
        json.loads(source)
        return "JSON"
    if suffix == ".toml":
        tomllib.loads(source)
        return "TOML"
    if suffix in {".yaml", ".yml"}:
        _check_yaml(path)
        return "YAML"
    raise ValueError(f"unsupported source type: {suffix}")


def main(argv: list[str]) -> int:
    paths = [Path(argument).resolve() for argument in argv] if argv else _tracked_sources()
    counts = dict.fromkeys(("YAML", "Python", "shell", "JSON", "TOML"), 0)
    failed = False
    for path in paths:
        if path.suffix not in SUPPORTED_SUFFIXES:
            print(f"{path}: unsupported source type", file=sys.stderr)
            failed = True
            continue
        try:
            kind = _check(path)
        except (
            OSError,
            SyntaxError,
            ValueError,
            json.JSONDecodeError,
            tomllib.TOMLDecodeError,
            yaml.YAMLError,
            subprocess.CalledProcessError,
        ) as error:
            detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) else error
            print(f"{path}: {detail}", file=sys.stderr)
            failed = True
        else:
            counts[kind] += 1
    if failed:
        return 1
    print(
        "Source syntax passed: "
        + ", ".join(f"{name}={count}" for name, count in counts.items())
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
