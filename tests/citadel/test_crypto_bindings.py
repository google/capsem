"""Native language bindings preserve provider ownership outside Rust."""

from __future__ import annotations

import ast
import re
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
PYTHON_OWNER = "build_system/builder/crypto.py"
NODE_OWNER = "sdk/typescript/tools/crypto.mjs"
PROVIDERS = {"hashlib", "blake3", "xxhash", "hmac", "secrets", "cryptography", "Crypto"}
BINDINGS_RATIONALE = (
    "Provider selection belongs to capsem-crypto for Rust and the owned native "
    "bindings for build/package tooling. Bootstrap cannot import first-party "
    "modules before its source-keyed re-exec. Independent test references and "
    "portable installed/guest scripts keep their native bindings. See "
    "crates/capsem-crypto/README.md."
)


def python_providers(source: str) -> set[str]:
    found = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            found.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            found.add(node.module.split(".")[0])
    return found & PROVIDERS


def node_provider(source: str) -> bool:
    return bool(
        re.search(
            r"(?:from\s*|require\s*\(|import\s*\()[\"'](?:node:)?crypto[\"']", source
        )
    )


def test_native_bindings_own_first_party_tooling_providers():
    paths = subprocess.run(
        ["git", "ls-files", "--", "build_system", "images/ci", "sdk", "mcp", "web", "tests/helpers"],
        cwd=ROOT,
        check=True,
        text=True,
        capture_output=True,
    ).stdout.splitlines()
    violations = []
    for name in paths:
        path = ROOT / name
        if (
            path.suffix == ".py"
            and name.startswith(("build_system/", "images/ci/"))
            and "/tests/" not in name
        ):
            if name == PYTHON_OWNER:
                continue
            providers = python_providers(path.read_text())
            if name == "build_system/builder/gatelaunch.py":
                providers -= {"hashlib"}  # Exact stdlib-only source-key bootstrap.
            if providers:
                violations.append(f"{name}: {sorted(providers)}")
        if (
            name.startswith("tests/helpers/") and path.suffix == ".py"
            and "xxhash" in python_providers(path.read_text())
        ):
            violations.append(f"{name}: xxhash outside canonical owner")
        if (
            path.suffix in {".ts", ".js", ".mjs", ".svelte", ".astro"}
            and name != NODE_OWNER
            and node_provider(path.read_text())
        ):
            violations.append(f"{name}: Node crypto")
    assert not violations, "\n".join(violations) + "\n" + BINDINGS_RATIONALE


@pytest.mark.parametrize(
    "source",
    [
        "import xxhash as fast",
        "def f():\n from xxhash import xxh3_128_intdigest",
        "import hashlib as fast",
        "from hashlib import sha256 as fast",
        "import cryptography.hazmat.primitives.hashes",
        "def f():\n import secrets as hidden",
    ],
)
def test_python_binding_guard_handles_aliases_and_nested_imports(source):
    assert python_providers(source)


@pytest.mark.parametrize(
    "source",
    [
        "import {createHash as fast} from 'node:crypto';",
        "const fast = require('crypto');",
        "const fast = await import('node:crypto');",
    ],
)
def test_node_binding_guard_handles_static_and_dynamic_imports(source):
    assert node_provider(source)
