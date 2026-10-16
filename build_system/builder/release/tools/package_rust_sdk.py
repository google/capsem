"""Package and verify the registry SDK's complete library graph with Cargo.

Cargo's multi-package verification resolves unpublished siblings together.
Checking only the SDK inside this workspace misses absent packaged build inputs.
Both local qualification and release publication use these exact archives.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from graphlib import CycleError, TopologicalSorter
from pathlib import Path
from typing import Any

from capsem_builder.gate.config import load


def publication_order(
    packages: list[dict[str, Any]], sdk_manifest: Path, root: Path
) -> list[dict[str, Any]]:
    """Resolve Cargo's real local edges, including optional/build/renamed ones."""
    root = root.resolve()
    sdk_manifest = sdk_manifest.resolve()
    by_manifest = {Path(row["manifest_path"]).resolve(): row for row in packages}
    if len(by_manifest) != len(packages):
        raise ValueError("duplicate Cargo package manifest")
    if sdk_manifest not in by_manifest:
        raise ValueError("SDK missing from Cargo metadata")
    version = by_manifest[sdk_manifest]["version"]
    graph: dict[Path, set[Path]] = {}
    pending = [sdk_manifest]
    while pending:
        manifest = pending.pop()
        if manifest in graph:
            continue
        if not manifest.is_relative_to(root) or manifest not in by_manifest:
            raise ValueError(f"local SDK dependency outside the workspace: {manifest}")
        package = by_manifest[manifest]
        if package["version"] != version:
            raise ValueError(f"SDK dependency version differs: {package['name']}")
        allowed = package.get("publish")
        if allowed is not None and "crates-io" not in allowed:
            raise ValueError(f"SDK dependency cannot publish to crates.io: {package['name']}")
        graph[manifest] = {
            (Path(dependency["path"]) / "Cargo.toml").resolve()
            for dependency in package["dependencies"]
            if dependency.get("path") and dependency.get("kind") != "dev"
        }
        pending.extend(sorted(graph[manifest]))
    try:
        return [by_manifest[path] for path in TopologicalSorter(graph).static_order()]
    except CycleError as error:
        raise ValueError("cyclic SDK publication dependencies") from error


def main() -> None:
    config = load(Path.cwd())
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
        cwd=config.root,
    ))
    packages = publication_order(metadata["packages"], config.path(config.sdk_rust.manifest), config.root)
    selected = [argument for package in packages for argument in ("--package", package["name"])]
    subprocess.run(["cargo", "package", "--locked", "--offline", *selected], cwd=config.root, check=True)
    output = config.path(config.sdk_rust.build_output)
    output.mkdir(parents=True, exist_ok=True)
    for package in packages:
        archive = f"{package['name']}-{package['version']}.crate"
        shutil.copy2(Path(metadata["target_directory"]) / "package" / archive, output / archive)


if __name__ == "__main__":
    main()
