"""Publication rejects stale package metadata and unqualified crate sources."""

import io
import json
import tarfile
import zipfile
from pathlib import Path

import pytest
from capsem_builder.release.tools.client_package_archives import Package, validate_archive

SOURCE = "a" * 40


def tar(path: Path, files: dict[str, bytes]) -> Path:
    with tarfile.open(path, "w:gz") as archive:
        for name, value in files.items():
            member = tarfile.TarInfo(name)
            member.size = len(value)
            archive.addfile(member, io.BytesIO(value))
    return path


@pytest.mark.parametrize("mutation", [None, "name", "version", "repository", "dependency"])
def test_npm_package_metadata_must_match_its_source_cohort(tmp_path: Path, mutation: str | None) -> None:
    metadata = {
        "name": "@capsem/mcp", "version": "0.7.1", "license": "Apache-2.0",
        "repository": {"type": "git", "url": "git+https://github.com/google/capsem.git", "directory": "mcp/typescript"},
        "dependencies": {"@capsem/sdk": "0.7.1"},
    }
    if mutation == "repository":
        metadata["repository"] = {"url": "https://example.invalid/other"}
    elif mutation == "dependency":
        metadata["dependencies"] = {"@capsem/sdk": "workspace:0.7.1"}
    elif mutation:
        metadata[mutation] = "wrong"
    archive = tar(tmp_path / "mcp.tgz", {"package/package.json": json.dumps(metadata).encode()})
    package = Package("npm", "@capsem/mcp", "0.7.1", "mcp/typescript", (archive,), (("@capsem/sdk", "0.7.1"),))
    if mutation:
        with pytest.raises(ValueError):
            validate_archive(package, archive, SOURCE)
    else:
        validate_archive(package, archive, SOURCE)


@pytest.mark.parametrize("mutation", [None, "source", "dirty", "version"])
def test_crate_must_record_the_exact_clean_qualified_source(tmp_path: Path, mutation: str | None) -> None:
    vcs = {"git": {"sha1": SOURCE, "dirty": False}}
    version = "0.7.1"
    if mutation == "source":
        vcs["git"]["sha1"] = "b" * 40
    elif mutation == "dirty":
        vcs["git"]["dirty"] = True
    elif mutation == "version":
        version = "0.7.0"
    prefix = "capsem-sdk-0.7.1"
    archive = tar(tmp_path / "sdk.crate", {
        f"{prefix}/Cargo.toml": f'[package]\nname="capsem-sdk"\nversion="{version}"\n'.encode(),
        f"{prefix}/.cargo_vcs_info.json": json.dumps(vcs).encode(),
    })
    package = Package("crates", "capsem-sdk", "0.7.1", "sdk/rust", (archive,))
    if mutation:
        with pytest.raises(ValueError):
            validate_archive(package, archive, SOURCE)
        if mutation == "dirty":
            validate_archive(package, archive, SOURCE, allow_dirty=True)
    else:
        validate_archive(package, archive, SOURCE)


@pytest.mark.parametrize("version", ["0.7.1", "0.7.0"])
def test_wheel_metadata_not_the_filename_proves_the_selected_version(tmp_path: Path, version: str) -> None:
    archive = tmp_path / "capsem-0.7.1-py3-none-any.whl"
    with zipfile.ZipFile(archive, "w") as wheel:
        wheel.writestr("capsem-0.7.1.dist-info/METADATA", f"Name: capsem\nVersion: {version}\n")
    package = Package("pypi", "capsem", "0.7.1", "sdk/python", (archive,))
    if version != package.version:
        with pytest.raises(ValueError):
            validate_archive(package, archive, SOURCE)
    else:
        validate_archive(package, archive, SOURCE)
