"""Fail-closed pnpm dependency audit using npm's Bulk Advisory endpoint."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import urllib.request
from collections import defaultdict
from pathlib import Path
from typing import Any

DEFAULT_ENDPOINT = "https://registry.npmjs.org/-/npm/v1/security/advisories/bulk"
DEFAULT_PROJECT_DIRS = (
    Path("web/app"),
    Path("web/docs"),
    Path("web/marketing"),
    Path("build_system/release_site"),
)


def collect_versions(tree: object) -> dict[str, list[str]]:
    versions: defaultdict[str, set[str]] = defaultdict(set)

    def visit(node: object) -> None:
        if not isinstance(node, dict):
            return
        name = node.get("from") or node.get("name")
        version = node.get("version")
        if isinstance(name, str) and name and isinstance(version, str) and version:
            versions[name].add(version)
        for field in ("dependencies", "devDependencies", "optionalDependencies"):
            children = node.get(field)
            if isinstance(children, dict):
                for child in children.values():
                    visit(child)

    if not isinstance(tree, list):
        raise ValueError("pnpm list output must be a JSON array")
    for root in tree:
        visit(root)
    if not versions:
        raise ValueError("pnpm list returned no dependency versions")
    return {name: sorted(found) for name, found in sorted(versions.items())}


def advisory_failures(response: object) -> list[str]:
    if not isinstance(response, dict):
        raise ValueError("npm bulk advisory response must be a JSON object")
    failures: list[str] = []
    for package, advisories in sorted(response.items()):
        if not isinstance(package, str) or not isinstance(advisories, list):
            raise ValueError("npm bulk advisory response has an invalid package entry")
        for advisory in advisories:
            if not isinstance(advisory, dict):
                raise ValueError(f"npm bulk advisory for {package} is not an object")
            severity = advisory.get("severity", "unknown")
            title = advisory.get("title", "untitled advisory")
            vulnerable = advisory.get("vulnerable_versions", "unknown versions")
            url = advisory.get("url", "missing advisory URL")
            failures.append(f"{package}: {severity}: {title} ({vulnerable}) {url}")
    return failures


def load_dependency_tree(project_dir: Path) -> Any:
    result = subprocess.run(
        ["pnpm", "list", "--json", "--depth", "Infinity"],
        cwd=project_dir,
        check=True,
        capture_output=True,
        text=True,
        timeout=120,
    )
    return json.loads(result.stdout)


def fetch_advisories(endpoint: str, versions: dict[str, list[str]]) -> Any:
    body = json.dumps(versions, separators=(",", ":"), sort_keys=True).encode("utf-8")
    request = urllib.request.Request(
        endpoint,
        data=body,
        headers={
            "Accept": "application/json",
            "Content-Type": "application/json",
            "User-Agent": "capsem-pnpm-bulk-audit",
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project-dir", action="append", type=Path)
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    args = parser.parse_args()

    project_dirs = args.project_dir or list(DEFAULT_PROJECT_DIRS)
    failed = False
    for project_dir in project_dirs:
        versions = collect_versions(load_dependency_tree(project_dir))
        advisories = fetch_advisories(args.endpoint, versions)
        failures = advisory_failures(advisories)
        if failures:
            failed = True
            for failure in failures:
                print(f"error: {project_dir}: {failure}", file=sys.stderr)
            continue
        version_count = sum(len(found) for found in versions.values())
        print(
            f"npm bulk audit clean: {project_dir}: {len(versions)} packages, "
            f"{version_count} installed versions"
        )
    return 1 if failed else 0


def entrypoint() -> int:
    """Render operational failures as the command's fail-closed exit status."""
    try:
        return main()
    except (OSError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"npm bulk audit failed: {error}", file=sys.stderr)
        return 1
