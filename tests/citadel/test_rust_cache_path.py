"""Hosted Rust caches must follow Cargo's policy-owned target directory."""

from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
CACHE_ACTION = ROOT / ".github" / "actions" / "rust-cache" / "action.yaml"
LOCAL_ACTION = "./.github/actions/rust-cache"


def test_workflows_use_one_rust_cache_adapter() -> None:
    uses: list[tuple[str, str]] = []
    for path in WORKFLOWS.glob("*.yaml"):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for job in (document.get("jobs") or {}).values():
            for step in job.get("steps") or ():
                action = step.get("uses", "")
                if "rust-cache" in action:
                    uses.append((path.name, action))

    assert uses
    assert all(action == LOCAL_ACTION for _, action in uses), uses


def test_rust_cache_adapter_owns_the_real_cargo_target() -> None:
    action = yaml.safe_load(CACHE_ACTION.read_text(encoding="utf-8"))
    step = action["runs"]["steps"][0]

    assert step["uses"] == "Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32"
    assert step["with"]["workspaces"] == ". -> cache/target/cargo"
