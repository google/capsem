"""Select registry archives by source metadata, never by newest cached filename."""

from __future__ import annotations

import email
import json
import subprocess
import tarfile
import tomllib
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from packaging.requirements import Requirement
from packaging.utils import canonicalize_name

from ...gate.config import GateConfig
from .package_rust_sdk import cargo_metadata, publication_order
from .verify_immutable_publication import file_identity


@dataclass(frozen=True)
class Package:
    registry: Literal["pypi", "npm", "crates"]
    name: str
    version: str
    project: str
    archives: tuple[Path, ...]
    dependencies: tuple[tuple[str, str], ...] = ()


def _tar_member(path: Path, member: str) -> bytes:
    with tarfile.open(path) as archive:
        entry = archive.getmember(member)
        if not entry.isfile():
            raise ValueError(f"package metadata must be a regular file: {member}")
        stream = archive.extractfile(entry)
        if stream is None:
            raise ValueError(f"missing package metadata: {member}")
        return stream.read()


def validate_archive(package: Package, archive: Path, source: str, *, allow_dirty: bool = False) -> None:
    """Require the exact selected metadata before any registry credential is used."""
    if archive.is_symlink() or not archive.is_file():
        raise ValueError(f"missing or unsafe publication archive: {archive}")
    if package.registry == "npm":
        metadata = json.loads(_tar_member(archive, "package/package.json"))
        repository = {"type": "git", "url": "git+https://github.com/google/capsem.git", "directory": package.project}
        if metadata.get("repository") != repository or metadata.get("license") != "Apache-2.0":
            raise ValueError(f"npm source metadata mismatch: {package.name}")
        if metadata.get("name") != package.name or metadata.get("version") != package.version:
            raise ValueError(f"npm package identity mismatch: {package.name}")
        for name, version in package.dependencies:
            if metadata.get("dependencies", {}).get(name) != version:
                raise ValueError(f"npm dependency mismatch: {package.name} -> {name}")
    elif package.registry == "crates":
        prefix = f"{package.name}-{package.version}"
        metadata = tomllib.loads(_tar_member(archive, f"{prefix}/Cargo.toml").decode())["package"]
        vcs = json.loads(_tar_member(archive, f"{prefix}/.cargo_vcs_info.json"))["git"]
        if metadata.get("name") != package.name or metadata.get("version") != package.version:
            raise ValueError(f"crate identity mismatch: {package.name}")
        if vcs.get("sha1") != source or (not allow_dirty and vcs.get("dirty", False) is not False):
            raise ValueError(f"crate is not the exact clean release source: {package.name}")
    else:
        if archive.suffix == ".whl":
            with zipfile.ZipFile(archive) as wheel:
                names = [name for name in wheel.namelist() if name.endswith(".dist-info/METADATA")]
                if len(names) != 1:
                    raise ValueError("wheel must contain exactly one metadata record")
                raw = wheel.read(names[0])
        else:
            prefix = f"{package.name.replace('-', '_')}-{package.version}"
            raw = _tar_member(archive, f"{prefix}/PKG-INFO")
        metadata = email.message_from_bytes(raw)
        if canonicalize_name(metadata.get("Name", "")) != canonicalize_name(package.name) or metadata.get("Version") != package.version:
            raise ValueError(f"Python package identity mismatch: {package.name}")
        requirements = [Requirement(value) for value in metadata.get_all("Requires-Dist", [])]
        for name, version in package.dependencies:
            if not any(canonicalize_name(req.name) == canonicalize_name(name) and version in req.specifier for req in requirements):
                raise ValueError(f"Python dependency mismatch: {package.name} -> {name}")


def collect(config: GateConfig, source: str, *, allow_dirty: bool = False) -> tuple[Package, ...]:
    """The source-owned SDK graph and external adapters, in dependency order."""
    version = tomllib.loads(config.path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    metadata = cargo_metadata(config.root)
    rust = publication_order(metadata["packages"], config.path(config.sdk_rust.manifest), config.root)
    packages = []
    for row in rust:
        project = Path(row["manifest_path"]).resolve().parent.relative_to(config.root.resolve()).as_posix()
        dependencies = tuple((dep["name"], version) for dep in row["dependencies"] if dep.get("path") and dep.get("kind") != "dev")
        packages.append(Package("crates", row["name"], row["version"], project,
                                (config.path(config.sdk_rust.build_output) / f"{row['name']}-{version}.crate",), dependencies))
    python_sdk = tomllib.loads(config.path(config.sdk_python.manifest).read_text())["project"]
    for settings, dependencies in ((config.sdk_python, ()), (config.integrations_inspect_ai, ((python_sdk["name"], version),))):
        project = tomllib.loads(config.path(settings.manifest).read_text())["project"]
        if settings is config.sdk_python and project["version"] != version:
            raise ValueError("Python SDK differs from native release version")
        basename = f"{project['name'].replace('-', '_')}-{project['version']}"
        packages.append(Package("pypi", project["name"], project["version"], settings.project, (
            config.path(settings.build_output) / f"{basename}-py3-none-any.whl",
            config.path(settings.build_output) / f"{basename}.tar.gz",
        ), dependencies))
    sdk_name = json.loads(config.path(config.sdk_typescript.manifest).read_text())["name"]
    for settings, dependencies in ((config.sdk_typescript, ()), (config.mcp_typescript, ((sdk_name, version),))):
        project = json.loads((config.path(settings.project) / "package.json").read_text())
        if project["version"] != version:
            raise ValueError("npm package differs from native release version")
        basename = project["name"].removeprefix("@").replace("/", "-")
        packages.append(Package("npm", project["name"], version, settings.project,
                                (config.path(settings.build_output) / f"{basename}-{version}.tgz",), dependencies))
    for package in packages:
        if package.registry != "pypi" and package.version != version:
            raise ValueError(f"SDK library differs from native release version: {package.name}")
        for archive in package.archives:
            validate_archive(package, archive, source, allow_dirty=allow_dirty)
    sdk = next(package for package in packages if package.registry == "npm" and package.name == sdk_name)
    tested_sdk = config.path(config.mcp_typescript.build_output) / sdk.archives[0].name
    if file_identity(tested_sdk) != file_identity(sdk.archives[0]):
        raise ValueError("MCP acceptance used a different SDK archive from publication")
    return tuple(packages)


def main() -> None:
    from ...gate.config import load

    config = load(Path.cwd())
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=config.root, text=True).strip()
    # Ordinary local qualification supports uncommitted work, whose source
    # digest the gate records. Publication calls collect's clean default.
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=config.root, text=True).strip())
    for package in collect(config, source, allow_dirty=dirty):
        print(f"Verified source archive: {package.registry} {package.name} {package.version}")


if __name__ == "__main__":
    main()
