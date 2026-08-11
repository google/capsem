"""Checked-in first-party scripts stay small enough to remain reviewable.

The scope is deliberately Git-tracked source under configured first-party
roots. Generated output and dependency/vendor trees therefore never enter the
inventory, while suffix-less installer scripts are still recognized by their
shebang.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

from capsem.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
SETTINGS = CONFIG.boundary.scripts


def _tracked_script_line_counts(
    root: Path, *, roots: tuple[str, ...], suffixes: tuple[str, ...]
) -> dict[str, int]:
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "--", *roots],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    inventory = {}
    for raw_path in tracked:
        if not raw_path:
            continue
        relative = Path(raw_path.decode())
        source = (root / relative).read_text(encoding="utf-8")
        if relative.suffix in suffixes or source.startswith("#!"):
            inventory[relative.as_posix()] = len(source.splitlines())
    return dict(sorted(inventory.items()))


def test_oversized_scripts_match_the_exact_debt_ratchet() -> None:
    inventory = _tracked_script_line_counts(
        PROJECT_ROOT, roots=SETTINGS.roots, suffixes=SETTINGS.suffixes
    )
    assert len(inventory) > 50, "scanned too few scripts to trust this guard"

    oversized = {
        path: lines for path, lines in inventory.items() if lines > SETTINGS.max_lines
    }
    expected = dict(SETTINGS.oversized_line_counts)

    assert oversized == expected, (
        f"new scripts may not exceed {SETTINGS.max_lines} lines, and existing "
        "oversized scripts have exact ratchets: split growth before merging; "
        "when one shrinks, lower or remove its config/gate.toml entry.\n"
        f"expected: {expected}\nactual: {oversized}"
    )


def test_only_tracked_first_party_programs_enter_the_inventory(tmp_path: Path) -> None:
    (tmp_path / "scripts").mkdir()
    (tmp_path / "vendor").mkdir()
    (tmp_path / "scripts" / "module.py").write_text("one\ntwo\n", encoding="utf-8")
    (tmp_path / "scripts" / "installer").write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    (tmp_path / "scripts" / "data.xml").write_text("<data />\n", encoding="utf-8")
    (tmp_path / "scripts" / "generated.py").write_text("generated\n", encoding="utf-8")
    (tmp_path / "vendor" / "tool.py").write_text("vendored\n", encoding="utf-8")

    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    subprocess.run(
        ["git", "add", "scripts/module.py", "scripts/installer", "scripts/data.xml", "vendor"],
        cwd=tmp_path,
        check=True,
    )

    assert _tracked_script_line_counts(
        tmp_path, roots=("scripts",), suffixes=(".py", ".sh")
    ) == {"scripts/installer": 2, "scripts/module.py": 2}
