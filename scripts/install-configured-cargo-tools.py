#!/usr/bin/env python3
"""Install a selected subset of the exact Cargo tools in gate config."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[1]


def _probe(argv: tuple[str, ...]) -> str:
    """What the tool reports, or nothing when it is not there to report it.

    `check=False` covers a tool that runs and exits non-zero. It does nothing
    for one that is not on PATH, which never runs at all: `subprocess.run`
    raises first. Both answers mean the same thing to the caller -- this is
    not the expected version -- and a fresh runner has none of these, which
    is the entire reason this script exists.
    """
    try:
        result = subprocess.run(argv, capture_output=True, text=True, check=False)
    except (FileNotFoundError, NotADirectoryError, PermissionError):
        return ""
    return (result.stdout + result.stderr).strip()


def main(argv: list[str]) -> int:
    config = gate_config.load(ROOT)
    configured = {tool.name: tool for tool in config.toolchain.crates}
    if not argv or len(argv) != len(set(argv)):
        print("usage: install-configured-cargo-tools.py TOOL [TOOL ...]", file=sys.stderr)
        return 2
    unknown = sorted(set(argv) - configured.keys())
    if unknown:
        print(f"unknown configured Cargo tools: {', '.join(unknown)}", file=sys.stderr)
        return 2
    for name in argv:
        tool = configured[name]
        if _probe(tool.probe).startswith(tool.expected):
            continue
        subprocess.run(tool.install, check=True)
        actual = _probe(tool.probe)
        if not actual.startswith(tool.expected):
            print(
                f"{name} did not provide {tool.expected}: {actual or '<no version output>'}",
                file=sys.stderr,
            )
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
