"""The Rust SDK shares workspace checks, package ratchets and published coverage."""

from __future__ import annotations

import re
import tomllib
from copy import deepcopy

import pytest
from citadel.test_sdk_ci import _documents
from citadel.test_sdk_quality import CONFIG, ROOT, SDK_RATIONALE


def _assert_owners(manifest: dict, floors: dict, ci: dict, coverage: dict) -> None:
    assert manifest["package"]["name"] == "capsem-sdk", SDK_RATIONALE
    assert manifest["lints"]["workspace"] is True, SDK_RATIONALE
    assert all(manifest.get("lib", {}).get(key, True) for key in ("test", "doctest")), SDK_RATIONALE
    assert floors.get("capsem-sdk", 0) >= 90, SDK_RATIONALE
    local = {name for name in manifest["dependencies"] if name.startswith("capsem")}
    assert local == {"capsem-api"}, "SDK dependencies must not grant service/core or local discovery access"
    steps = ci["jobs"]["test"]["steps"]
    commands = next(step["run"] for step in steps if step.get("name") == "Unit tests with coverage")
    for command in commands.splitlines():
        if "cargo llvm-cov" in command:
            assert "-p capsem-sdk" in command and "--exclude" not in command, SDK_RATIONALE
    for flag in ("unit", "linux-unit"):
        assert "sdk/rust/src/**" in coverage["flags"][flag]["paths"], SDK_RATIONALE
    component = next(item for item in coverage["component_management"]["individual_components"]
                     if item["component_id"] == "rust-sdk")
    assert component["paths"] == ["sdk/rust/src/**"], SDK_RATIONALE


def _settings() -> tuple[dict, dict, dict, dict]:
    _, ci, coverage = _documents()
    return (tomllib.loads((ROOT / "sdk/rust/Cargo.toml").read_text()),
            dict(CONFIG.modules.rust_coverage_crate_floors), ci, coverage)


def test_rust_sdk_has_native_coverage_and_ci_owners() -> None:
    _assert_owners(*_settings())
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    assert "sdk/rust" in workspace["members"], SDK_RATIONALE
    for command in (CONFIG.modules.rust_coverage, CONFIG.modules.rust_doctests):
        assert "--workspace" in command and "--exclude" not in command, SDK_RATIONALE
    for path in (ROOT / "sdk/rust/src").rglob("*.rs"):
        assert not re.search(r"#\s*\[\s*(?:allow|expect|ignore|coverage)\b|cfg.*coverage", path.read_text()), SDK_RATIONALE


@pytest.mark.parametrize("mutation", ["floor", "lints", "tests", "doctests", "runtime", "ci", "report", "paths", "generated"])
def test_rust_sdk_oversight_weakening_is_rejected(mutation: str) -> None:
    manifest, floors, ci, coverage = deepcopy(_settings())
    match mutation:
        case "floor":
            floors.pop("capsem-sdk")
        case "lints":
            manifest["lints"]["workspace"] = False
        case "tests" | "doctests":
            manifest["lib"] = {"test" if mutation == "tests" else "doctest": False}
        case "runtime":
            manifest["dependencies"]["capsem-core"] = {}
        case "ci" | "report":
            for step in ci["jobs"]["test"]["steps"]:
                if step.get("name") == "Unit tests with coverage":
                    lines = step["run"].splitlines()
                    index = 1 if mutation == "ci" else 2
                    lines[index] = lines[index].replace(" -p capsem-sdk", "")
                    step["run"] = "\n".join(lines)
        case "paths":
            coverage["flags"]["unit"]["paths"].remove("sdk/rust/src/**")
        case "generated":
            next(item for item in coverage["component_management"]["individual_components"]
                 if item["component_id"] == "rust-sdk")["paths"] = ["sdk/rust/src/transport.rs"]
    with pytest.raises(AssertionError):
        _assert_owners(manifest, floors, ci, coverage)
