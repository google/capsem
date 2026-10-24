"""The input key of an official image: everything its build reads, as one digest.

The publication workflow tags each build with its key and skips a build whose
key is already published. So the key must change whenever an input does --
a file's bytes, its name, its executable bit, or (for an agent image) the
base digest it builds FROM -- and must not change for anything else, such as
the order a checkout happened to write files in.

Usage: inputkey.py <path>... [--with <value>]...
"""

from __future__ import annotations

import argparse
import re
import stat
import sys
from collections.abc import Sequence
from pathlib import Path

from capsem_builder import crypto as hashlib

ROOT = Path(__file__).resolve().parents[2]
#: Bumped when the key's construction changes, so no old tag is mistaken for
#: a build of the same inputs under the new rule.
VERSION = b"capsem-official-image-input-key-v1"
#: Files beside an image that its build never reads: the qualification
#: manifest says how the built image is tested, so editing it re-qualifies the
#: image (tests/qualification) instead of rebuilding it.
NOT_BUILD_INPUTS = frozenset({"qualify.toml"})
# Derived generations include filesystem evidence; base-only v1 stays intact.
DERIVED_DOMAIN = "capsem-derived-image-producer-v2"
DERIVED_PATTERNS = (
    "images/ci/*.py", "build_system/builder/image/*.py",
    "build_system/builder/gate/*schema.py",
)
DERIVED_FILES = (
    ".github/workflows/images.yaml", "build_system/builder/gate/__init__.py",
    "build_system/builder/__init__.py", "build_system/builder/crypto.py", "build_system/builder/privateimages.py",
    "build_system/builder/gate/config.py", "build_system/builder/gate/host.py",
    "build_system/builder/gate/configschema.py", "build_system/builder/gate/buildschema.py",
    "build_system/builder/gate/errors.py", "build_system/builder/gate/auditfs.py",
    "build_system/builder/gate/exclusions.py", "build_system/builder/policy/dockerpolicy.py",
    "build_system/builder/release/releasechannel.py", "build_system/builder/release/obom.py",
    "build_system/builder/release/package_inventory.py", "build_system/pyproject.toml", "build_system/uv.lock",
    "build_system/builder/image/__init__.py", "build_system/builder/image/docker.py",
    "build_system/builder/image/config.py", "build_system/builder/image/models.py",
    "build_system/builder/image/schema.py", "build_system/builder/image/assettools.py",
)


def _files(path: Path) -> list[Path]:
    if path.is_file():
        return [path]
    files = sorted(
        entry for entry in path.rglob("*") if entry.is_file() and entry.name not in NOT_BUILD_INPUTS
    )
    if not files:
        raise ValueError(f"{path} holds no files, so it cannot key a build")
    return files


def input_key(paths: Sequence[Path], *, root: Path = ROOT, extra: Sequence[str] = ()) -> str:
    """Hex SHA-256 over every file under `paths`, then each `extra` value."""
    digest = hashlib.sha256(VERSION + b"\0")
    files = sorted({file for path in paths for file in _files(path)})
    for file in files:
        data = file.read_bytes()
        executable = bool(file.stat().st_mode & stat.S_IXUSR)
        for field in (
            b"file",
            file.relative_to(root).as_posix().encode(),
            b"x" if executable else b"-",
            str(len(data)).encode(),
        ):
            digest.update(field + b"\0")
        digest.update(data)
    for value in extra:
        digest.update(b"with\0" + value.encode() + b"\0")
    return digest.hexdigest()


def _contained(root: Path, relative: str, *, directory: bool = False) -> Path:
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts or str(path) != relative or not path.parts:
        raise ValueError(f"producer source path escapes its checkout: {relative}")
    selected = root / path
    for entry in (selected, *selected.parents):
        if entry == root:
            break
        if entry.is_symlink():
            raise ValueError(f"producer source must not be a symlink: {entry}")
    if not (selected.is_dir() if directory else selected.is_file()):
        raise ValueError(f"missing producer source: {relative}")
    return selected


def derived_key(name: str, base: str, *, root: Path = ROOT) -> str:
    """One validated official producer selection, shared by CI and native proof.

    Gate/guest config validation runs even on a cache hit. Only consumed data
    enters identity: mutable functional pins never key their own generation.
    Whole producer modules and imported config schemas remain conservative code
    inputs; unrelated gate commands are not dependencies of rootfs production.
    """
    import yaml
    from capsem_builder.gate import config as gate_config
    from capsem_builder.image import assettools, docker
    from capsem_builder.image.config import load_guest_config

    from images.ci.catalog import descriptions

    root = root.resolve(strict=True)
    if not re.fullmatch(r"[a-z][a-z0-9-]*", name) or not re.fullmatch(r"sha256:[0-9a-f]{64}", base):
        raise ValueError("derived producer requires an official image and original base digest")
    _contained(root, "config/gate.toml")
    if name not in descriptions(_contained(root, "images/catalog.toml")):
        raise ValueError("derived producer image is not official")
    config = gate_config.load(root)
    workflow = yaml.safe_load(_contained(root, ".github/workflows/images.yaml").read_text())
    registry = config.functional.native_images.base.repository.rsplit("/", 1)[0]
    reference = config.functional.reference_image
    if (workflow.get("env", {}).get("REGISTRY") != registry
        or reference.name != "dev" or reference.context != "images/dev"
        or reference.base_context != "images/base" or reference.repository != registry + "/dev"):
        raise ValueError("native declaration differs from the original workflow producer")
    selector = config.imagebuild.source_config
    source = _contained(root, selector, directory=True)
    for entry in source.rglob("*"):
        _contained(root, entry.relative_to(root).as_posix(), directory=entry.is_dir())
    build = load_guest_config(source).build
    # Reuse the producer's pure validation, never a weaker copied EROFS rule.
    docker.experimental_erofs_build_config({}, build.erofs)
    platforms = {arch.docker_platform for arch in build.architectures.values()}
    if platforms != {"linux/arm64", "linux/amd64"} or len(build.architectures) != 2:
        raise ValueError("derived producer requires both unique supported tool architectures")
    tools_file = _contained(root, build.asset_tools.dockerfile)
    extra = [DERIVED_DOMAIN, name, base, selector, build.erofs.model_dump_json()]
    extra.extend(assettools.image_tag(build, arch, root) for arch in sorted(build.architectures))
    paths = [_contained(root, f"images/{name}", directory=True), tools_file]
    paths.extend(_contained(root, name) for name in DERIVED_FILES)
    for pattern in DERIVED_PATTERNS:
        found = sorted(root.glob(pattern))
        if not found:
            raise ValueError(f"missing producer source pattern: {pattern}")
        paths.extend(found)
    for path in paths:
        if path.is_dir():
            for entry in path.rglob("*"):
                _contained(root, entry.relative_to(root).as_posix(), directory=entry.is_dir())
    for file in {file for path in paths for file in _files(path)}:
        _contained(root, file.relative_to(root).as_posix())
    return input_key(paths, root=root, extra=extra)


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="*", type=Path)
    parser.add_argument("--derived", metavar="IMAGE")
    parser.add_argument("--with", dest="extra", action="append", default=[])
    args = parser.parse_args(argv)
    if args.derived is not None:
        if args.paths or len(args.extra) != 1:
            parser.error("--derived accepts only one original base --with value, never path overrides")
        print(derived_key(args.derived, args.extra[0], root=ROOT))
        return
    if not args.paths:
        parser.error("base/file keys require source paths")
    paths = [path if path.is_absolute() else ROOT / path for path in args.paths]
    print(input_key(paths, extra=args.extra))


if __name__ == "__main__":
    sys.path.insert(0, str(ROOT))
    main()
