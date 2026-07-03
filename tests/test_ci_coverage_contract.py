"""CI coverage contracts for workspace crates and binary targets."""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class WorkspacePackage:
    name: str
    path: Path
    binary_targets: tuple[str, ...]


def test_workspace_crates_and_bins_enumerated() -> None:
    packages = workspace_packages()
    ci = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    macos_coverage_packages = ci_coverage_packages(ci, job_name="test")

    missing_packages = sorted(set(packages) - macos_coverage_packages)
    assert not missing_packages, (
        "macOS CI cargo llvm-cov package list must include every workspace "
        f"crate; missing {missing_packages}"
    )

    release_binary_targets = {
        "capsem",
        "capsem-admin",
        "capsem-app",
        "capsem-bench-rs",
        "capsem-dns-proxy",
        "capsem-gateway",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-mcp-server",
        "capsem-mock-server",
        "capsem-net-proxy",
        "capsem-process",
        "capsem-pty-agent",
        "capsem-service",
        "capsem-sysutil",
        "capsem-tray",
        "capsem-tui",
    }
    discovered_binary_targets = {
        binary
        for package in packages.values()
        for binary in package.binary_targets
    }
    missing_binary_targets = sorted(release_binary_targets - discovered_binary_targets)
    assert not missing_binary_targets, (
        "Cargo metadata must enumerate every release binary target; missing "
        f"{missing_binary_targets}"
    )

    missing_binary_packages = sorted(
        package.name
        for package in packages.values()
        if package.binary_targets and package.name not in macos_coverage_packages
    )
    assert not missing_binary_packages, (
        "CI coverage must include every package that owns binary targets; "
        f"missing {missing_binary_packages}"
    )


def test_every_crate_in_codecov() -> None:
    packages = workspace_packages()
    codecov = (PROJECT_ROOT / "codecov.yml").read_text()

    missing = [
        package.name
        for package in sorted(packages.values(), key=lambda item: item.name)
        if f"{package.path}/" not in codecov
    ]
    assert not missing, (
        "codecov.yml component paths must include every Cargo workspace "
        f"crate path; missing {missing}"
    )


def workspace_packages() -> dict[str, WorkspacePackage]:
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=PROJECT_ROOT,
            text=True,
        )
    )
    workspace_members = set(metadata["workspace_members"])
    packages: dict[str, WorkspacePackage] = {}
    for package in metadata["packages"]:
        if package["id"] not in workspace_members:
            continue
        manifest_path = Path(package["manifest_path"])
        binary_targets = tuple(
            sorted(
                target["name"]
                for target in package["targets"]
                if "bin" in target["kind"]
            )
        )
        packages[package["name"]] = WorkspacePackage(
            name=package["name"],
            path=manifest_path.parent.relative_to(PROJECT_ROOT),
            binary_targets=binary_targets,
        )
    return packages


def ci_coverage_packages(ci: str, *, job_name: str) -> set[str]:
    job = workflow_job_block(ci, job_name)
    packages: set[str] = set()
    for command in re.findall(r"cargo llvm-cov (?:nextest|report)[^\n]+", job):
        packages.update(re.findall(r"-p ([A-Za-z0-9_-]+)", command))
    return packages


def workflow_job_block(workflow: str, name: str) -> str:
    lines = workflow.splitlines()
    start = next((i for i, line in enumerate(lines) if line == f"  {name}:"), None)
    assert start is not None, f"workflow job {name} not found"
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line.startswith("  ") and not line.startswith("    ") and line.endswith(":"):
            end = i
            break
    return "\n".join(lines[start:end])
