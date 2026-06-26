"""Coverage infrastructure must include every workspace crate."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _workspace_crates() -> set[str]:
    workspace = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text())["workspace"]
    crates: set[str] = set()
    for member in workspace["members"]:
        cargo_toml = PROJECT_ROOT / member / "Cargo.toml"
        package = tomllib.loads(cargo_toml.read_text())["package"]
        crates.add(package["name"])
    return crates


def _ci_coverage_crates(command_name: str) -> set[str]:
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text()
    crates: set[str] = set()
    for command in re.findall(rf"cargo llvm-cov {command_name}[^\n]+", ci):
        crates.update(re.findall(r"-p ([A-Za-z0-9_-]+)", command))
    return crates


def test_pr_coverage_commands_include_every_workspace_crate() -> None:
    workspace_crates = _workspace_crates()
    for command_name in ("nextest", "report"):
        missing = workspace_crates - _ci_coverage_crates(command_name)
        assert not missing, (
            f"CI cargo llvm-cov {command_name} commands must include every "
            f"workspace crate; missing {sorted(missing)}"
        )


def test_codecov_components_cover_every_workspace_crate_path() -> None:
    codecov = (PROJECT_ROOT / "codecov.yml").read_text()
    missing = [
        crate
        for crate in sorted(_workspace_crates())
        if f"crates/{crate}/" not in codecov
    ]
    assert not missing, (
        "codecov.yml component paths must mention every workspace crate; missing "
        f"{missing}"
    )
