#!/usr/bin/env python3
"""Run Winterfell against one exact installed binary/profile/asset cohort."""

from __future__ import annotations

import argparse
import importlib
import json
import os
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
REPORT_SCHEMA = "capsem.installed_winterfell.v1"
WINTERFELL_ROOT_ENV = {
    "binary_dir": "CAPSEM_WINTERFELL_BIN_DIR",
    "assets_dir": "CAPSEM_WINTERFELL_ASSETS_DIR",
    "profiles_dir": "CAPSEM_WINTERFELL_PROFILES_DIR",
}
WINTERFELL_TESTS = (
    "tests/capsem-mcp/test_winterfell_rw.py",
    "tests/capsem-mcp/test_winterfell_exec.py",
)


def _resolve_winterfell_artifact_roots(overrides: dict[str, str]) -> Any:
    sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    module = importlib.import_module("helpers.service")
    return module.resolve_winterfell_artifact_roots(overrides)


def parse_args(arguments: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--assets-dir", required=True, type=Path)
    parser.add_argument("--profiles-dir", required=True, type=Path)
    parser.add_argument("--evidence-out", required=True, type=Path)
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    args = parse_args(arguments)
    overrides = {
        WINTERFELL_ROOT_ENV["binary_dir"]: str(args.bin_dir),
        WINTERFELL_ROOT_ENV["assets_dir"]: str(args.assets_dir),
        WINTERFELL_ROOT_ENV["profiles_dir"]: str(args.profiles_dir),
    }
    roots = _resolve_winterfell_artifact_roots(overrides)
    environment = os.environ.copy()
    environment.update(overrides)
    command = [
        os.fspath(Path(sys.executable)),
        "-m",
        "pytest",
        *WINTERFELL_TESTS,
        "-q",
        "-p",
        "no:cacheprovider",
    ]
    result = subprocess.run(command, cwd=PROJECT_ROOT, env=environment, check=False)
    report = {
        "schema": REPORT_SCHEMA,
        "passed": result.returncode == 0,
        "roots": {
            "assets": str(roots.assets_dir),
            "binaries": str(roots.binary_dir),
            "profiles": str(roots.profiles_dir),
        },
    }
    args.evidence_out.parent.mkdir(parents=True, exist_ok=True)
    args.evidence_out.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
