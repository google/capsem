"""Partial publication is resumable only for identical immutable bytes."""

from pathlib import Path

import pytest
from capsem_builder import crypto
from capsem_builder.release.tools.client_package_archives import Package
from capsem_builder.release.tools.client_registry import published_files


def package(tmp_path: Path) -> Package:
    files = (tmp_path / "capsem-0.7.1.whl", tmp_path / "capsem-0.7.1.tar.gz")
    for path in files:
        path.write_bytes(path.name.encode())
    return Package("pypi", "capsem", "0.7.1", "sdk/python", files)


@pytest.mark.parametrize("mutation", [None, "digest", "extra", "version"])
def test_partial_pypi_publication_can_only_resume_matching_files(tmp_path: Path, mutation: str | None) -> None:
    selected = package(tmp_path)
    wheel = selected.archives[0]
    payload = {"info": {"name": "capsem", "version": "0.7.1"}, "urls": [{
        "filename": wheel.name, "url": "https://files.pythonhosted.org/expected.whl",
        "digests": {"sha256": crypto.sha256(wheel.read_bytes()).hexdigest()},
    }]}
    if mutation == "digest":
        payload["urls"][0]["digests"]["sha256"] = "0" * 64
    elif mutation == "extra":
        payload["urls"].append({"filename": "unexpected.whl"})
    elif mutation == "version":
        payload["info"]["version"] = "0.7.0"
    if mutation:
        with pytest.raises(ValueError):
            published_files(selected, payload)
    else:
        assert published_files(selected, payload) == {wheel: "https://files.pythonhosted.org/expected.whl"}


@pytest.mark.parametrize("url", ["file:///private/value", "http://registry.npmjs.org/sdk.tgz", "https://unrelated.invalid/sdk.tgz"])
def test_npm_metadata_cannot_substitute_a_local_or_unrelated_download(tmp_path: Path, url: str) -> None:
    archive = tmp_path / "sdk.tgz"
    archive.write_bytes(b"sdk")
    selected = Package("npm", "@capsem/sdk", "0.7.1", "sdk/typescript", (archive,))
    with pytest.raises(ValueError):
        published_files(selected, {"name": selected.name, "version": selected.version, "dist": {"tarball": url}})


def test_existing_crate_version_must_match_the_prepared_archive_checksum(tmp_path: Path) -> None:
    archive = tmp_path / "sdk.crate"
    archive.write_bytes(b"sdk")
    selected = Package("crates", "capsem-sdk", "0.7.1", "sdk/rust", (archive,))
    version = {"crate": selected.name, "num": selected.version, "checksum": "0" * 64}
    with pytest.raises(ValueError):
        published_files(selected, {"version": version})
    version["checksum"] = crypto.sha256(archive.read_bytes()).hexdigest()
    assert list(published_files(selected, {"version": version})) == [archive]
