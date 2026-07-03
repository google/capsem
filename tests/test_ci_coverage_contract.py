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


@dataclass(frozen=True)
class CodecovComponent:
    component_id: str
    paths: tuple[str, ...]
    targets: tuple[str, ...]


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


def test_low_coverage_components_visible() -> None:
    components = codecov_components((PROJECT_ROOT / "codecov.yml").read_text())

    required = {
        "mcp-aggregator": "crates/capsem-mcp-aggregator/src/**",
        "mcp-builtin": "crates/capsem-mcp-builtin/src/**",
        "mcp-server": "crates/capsem-mcp/src/**",
        "mock-server": "crates/capsem-mock-server/src/**",
        "process": "crates/capsem-process/src/**",
    }

    missing = sorted(set(required) - set(components))
    assert not missing, (
        "low-coverage MCP/process crates need dedicated Codecov components; "
        f"missing {missing}"
    )

    wrong_paths = {
        component_id: components[component_id].paths
        for component_id, path in required.items()
        if components[component_id].paths != (path,)
    }
    assert not wrong_paths, (
        "low-coverage components must own exactly one crate path so weak "
        f"coverage cannot be hidden inside broad buckets; got {wrong_paths}"
    )

    weak_targets = {
        component_id: components[component_id].targets
        for component_id in required
        if "80%" not in components[component_id].targets
    }
    assert not weak_targets, (
        "low-coverage release-critical crates need explicit 80% project "
        f"targets; missing on {weak_targets}"
    )

    duplicated_paths = {
        path: sorted(
            component.component_id
            for component in components.values()
            if path in component.paths
        )
        for path in required.values()
    }
    duplicated_paths = {
        path: owners for path, owners in duplicated_paths.items() if len(owners) != 1
    }
    assert not duplicated_paths, (
        "low-coverage crate paths must not also appear in broad components; "
        f"duplicates {duplicated_paths}"
    )


def test_rust_coverage_includes_bins() -> None:
    commands = rust_coverage_commands(
        [
            PROJECT_ROOT / ".github" / "workflows" / "ci.yaml",
            PROJECT_ROOT / ".github" / "workflows" / "release.yaml",
            PROJECT_ROOT / "justfile",
        ]
    )
    unit_or_workspace_commands = {
        source: command
        for source, command in commands.items()
        if "--test" not in command
    }

    missing_bins = {
        source: command
        for source, command in unit_or_workspace_commands.items()
        if "--bins" not in command
    }
    assert not missing_bins, (
        "Rust unit/workspace coverage commands must include binary targets "
        f"with --bins; missing {missing_bins}"
    )


def test_release_critical_crates_are_reported() -> None:
    codecov = (PROJECT_ROOT / "codecov.yml").read_text()
    ci = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    release_site_package = json.loads((PROJECT_ROOT / "release-site" / "package.json").read_text())

    required_component_paths = {
        "crates/capsem-admin/src/**",
        "crates/capsem-app/src/**",
        "crates/capsem/src/**",
        "crates/capsem-gateway/src/**",
        "crates/capsem-mcp/src/**",
        "crates/capsem-mcp-aggregator/src/**",
        "crates/capsem-mcp-builtin/src/**",
        "crates/capsem-mock-server/src/**",
        "crates/capsem-process/src/**",
        "crates/capsem-service/src/**",
        "crates/capsem-tray/src/**",
        "crates/capsem-tui/src/**",
        "release-site/scripts/**",
        "release-site/src/**",
        "src/capsem/**",
    }
    missing_components = sorted(
        path for path in required_component_paths if path not in codecov
    )
    assert not missing_components, (
        "release-critical code paths must be visible in Codecov components; "
        f"missing {missing_components}"
    )

    required_uploads = {
        "codecov-linux.json",
        "codecov-unit.json",
        "codecov-integration.json",
        "codecov-python.xml",
        "frontend/coverage/coverage-final.json",
        "release-site/coverage/lcov.info",
    }
    uploaded_files = codecov_upload_files(ci)
    missing_uploads = sorted(required_uploads - uploaded_files)
    assert not missing_uploads, (
        "CI must upload coverage reports for release-critical Rust, Python, "
        f"frontend, and release-site code; missing {missing_uploads}"
    )

    scripts = release_site_package.get("scripts", {})
    assert "test:coverage" in scripts, "release-site must generate coverage metadata"
    assert "pnpm run test:coverage" in ci, (
        "release-site-build must run the release-site coverage script before "
        "the PR gate can pass"
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


def codecov_components(codecov: str) -> dict[str, CodecovComponent]:
    components: dict[str, CodecovComponent] = {}
    current_id: str | None = None
    paths: list[str] = []
    targets: list[str] = []
    in_paths = False

    def flush() -> None:
        if current_id is not None:
            components[current_id] = CodecovComponent(
                component_id=current_id,
                paths=tuple(paths),
                targets=tuple(targets),
            )

    for raw_line in codecov.splitlines():
        stripped = raw_line.strip()
        if raw_line.startswith("    - component_id: "):
            flush()
            current_id = stripped.split(": ", 1)[1]
            paths = []
            targets = []
            in_paths = False
            continue
        if current_id is None:
            continue
        if stripped == "paths:":
            in_paths = True
            continue
        if stripped.endswith(":") and stripped != "paths:":
            in_paths = False
        if in_paths and raw_line.startswith("        - "):
            paths.append(stripped[2:])
            continue
        if raw_line.startswith("          target: "):
            targets.append(stripped.split(": ", 1)[1])

    flush()
    return components


def rust_coverage_commands(paths: list[Path]) -> dict[str, str]:
    commands: dict[str, str] = {}
    for path in paths:
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            command = line.strip()
            if not command.startswith("cargo llvm-cov "):
                continue
            commands[f"{path.relative_to(PROJECT_ROOT)}:{line_number}"] = command
    return commands


def codecov_upload_files(workflow: str) -> set[str]:
    return {
        file.strip()
        for line in workflow.splitlines()
        if line.strip().startswith("files:")
        for file in line.strip().split(":", 1)[1].split(",")
        if file.strip()
    }


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
