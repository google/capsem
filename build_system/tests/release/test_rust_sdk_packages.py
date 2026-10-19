"""Registry SDK packages include their full local library graph in dependency order."""

from pathlib import Path

import pytest
from capsem_builder.release.tools.package_rust_sdk import publication_order


def package(name: str, *dependencies: str, kind: str | None = None) -> dict:
    return {
        "name": name,
        "version": "0.7.1",
        "manifest_path": f"/repo/{name}/Cargo.toml",
        "publish": None,
        "dependencies": [
            {"name": dependency, "path": f"/repo/{dependency}", "kind": kind}
            for dependency in dependencies
        ],
    }


def test_local_sdk_graph_is_dependency_ordered_and_excludes_applications() -> None:
    packages = [package("sdk", "api", "foundation"), package("unrelated-app"),
                package("foundation", "proto"), package("proto", "crypto"),
                package("api", "crypto"), package("crypto")]
    ordered = publication_order(packages, Path("/repo/sdk/Cargo.toml"), Path("/repo"))
    names = [row["name"] for row in ordered]
    assert set(names) == {"sdk", "api", "foundation", "proto", "crypto"}
    for row in ordered:
        for dependency in row["dependencies"]:
            assert names.index(dependency["name"]) < names.index(row["name"])


def test_dev_only_local_dependencies_are_not_published() -> None:
    sdk = package("sdk", "private-fixture", kind="dev")
    assert publication_order([sdk], Path(sdk["manifest_path"]), Path("/repo")) == [sdk]


@pytest.mark.parametrize("mutation", ["missing", "private", "version", "cycle", "outside"])
def test_invalid_publication_graphs_fail_before_packaging(mutation: str) -> None:
    sdk, dependency = package("sdk", "dependency"), package("dependency")
    packages = [sdk, dependency]
    if mutation == "missing":
        packages.pop()
    elif mutation == "private":
        dependency["publish"] = []
    elif mutation == "version":
        dependency["version"] = "0.7.0"
    elif mutation == "cycle":
        dependency["dependencies"] = package("dependency", "sdk")["dependencies"]
    else:
        dependency["manifest_path"] = "/elsewhere/dependency/Cargo.toml"
        sdk["dependencies"][0]["path"] = "/elsewhere/dependency"
    with pytest.raises(ValueError):
        publication_order(packages, Path(sdk["manifest_path"]), Path("/repo"))
