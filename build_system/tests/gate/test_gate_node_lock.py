"""Frozen Node installs must agree with the declared override owner."""

import json
from pathlib import Path

import yaml
from capsem_builder.gate.config import load

ROOT = Path(__file__).resolve().parents[3]


def test_every_gate_node_workspace_declares_its_locked_overrides():
    for workspace in load(ROOT).toolchain.node_workspaces:
        root = ROOT / workspace
        package = json.loads((root / "package.json").read_text())
        settings = yaml.safe_load((root / "pnpm-workspace.yaml").read_text()) or {}
        # pnpm 10's package.json configuration takes precedence when present.
        declared = package.get("pnpm", {}).get("overrides", settings.get("overrides", {}))
        locked = yaml.safe_load((root / "pnpm-lock.yaml").read_text()).get("overrides", {})
        assert declared == locked, (
            f"{workspace}: frozen pnpm install rejects declared overrides {declared} "
            f"that differ from the lockfile {locked}; preserve existing security pins"
        )
