"""Opt-in CI receipts for the two existing native image producers.

Not a pin, channel manifest, publisher or alternative release proof.
"""

from __future__ import annotations

import json
import platform
import subprocess
from pathlib import Path
from typing import Annotated, Literal

from capsem_builder.gate.configschema import Strict
from pydantic import StringConstraints

from . import producer_evidence, published_root

Section = Literal["debug_image", "reference_image"]
Commit = Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{40}$")]
Digest = Annotated[str, StringConstraints(pattern=r"^sha256:[0-9a-f]{64}$")]


class Source(Strict):
    commit: Commit
    tree: Commit


class Receipt(Strict):
    section: Section
    source: Source
    platform: Annotated[str, StringConstraints(pattern=r"^linux/(arm64|amd64)$")]
    image: str
    layout_key: Annotated[str, StringConstraints(pattern=r"^[A-Za-z0-9][A-Za-z0-9._-]*$")]
    subject: Digest
    config: Digest
    root: Digest
    root_integrity: Annotated[str, StringConstraints(pattern=r"^blake3:[0-9a-f]{64}$")]


def source(root: Path) -> Source:
    def git(*args):
        return subprocess.run(
            ["git", "-C", str(root), *args], capture_output=True, text=True, check=True, timeout=30
        ).stdout.strip()

    if git("status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("native image receipt requires clean source")
    return Source(commit=git("rev-parse", "HEAD"), tree=git("rev-parse", "HEAD^{tree}"))


def host_platform() -> str:
    machine = {"x86_64": "amd64", "aarch64": "arm64", "arm64": "arm64"}.get(platform.machine())
    return platform.system().lower() + "/" + str(machine)


def contained(root: Path, path: Path) -> Path:
    if ".." in path.parts or not path.is_relative_to(root) or any(c in str(path) for c in "\r\n"):
        raise ValueError("native candidate path traversal/outside owner")
    for part in (root, *path.parents):
        if part.is_relative_to(root) and part.is_symlink():
            raise ValueError("native candidate parent symlink")
    if path.is_symlink():
        raise ValueError("native candidate symlink")
    return path


def receipt_path(config, section: Section) -> Path:
    if section not in ("debug_image", "reference_image"):
        raise ValueError("native candidate section is not allowlisted")
    directory = config.path(config.functional.native_images.receipt_dir)
    return contained(config.root, directory / (section + ".json"))


def identity(image, digest: str, selected_platform: str, original: Source) -> Receipt:
    settings = image.settings()
    layout = image.layout_path(digest)
    contained(image.stage(), layout)
    if layout.parent != image.stage():
        raise ValueError("native image layout is not a direct managed generation")
    producer_evidence.ready(
        layout,
        settings.repository,
        selected_platform.split("/")[1],
        maximum=settings.evidence_max_bytes,
    )
    manifest = json.loads((layout / "blobs/sha256" / digest.split(":")[1]).read_bytes())
    root, _ = published_root.ready(layout)
    return Receipt(
        section=image.section,
        source=original,
        platform=selected_platform,
        image=settings.name,
        layout_key=layout.name,
        subject=digest,
        config=manifest["config"]["digest"],
        root=root["layers"][0]["digest"],
        root_integrity=root["annotations"]["org.capsem.rootfs.blake3"],
    )


def record(config, image, digest: str, selected_platform: str, original: Source) -> None:
    if (
        source(config.root) != original
        or host_platform() != selected_platform
        or selected_platform != config.functional.native_images.platform
    ):
        raise ValueError("native candidate source/platform changed during production")
    receipt = identity(image, digest, selected_platform, original)
    path = receipt_path(config, image.section)
    data = receipt.model_dump_json().encode()
    if len(data) > config.functional.native_images.receipt_max_bytes:
        raise ValueError("native candidate receipt budget exceeded")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".part")
    contained(config.root, temporary)
    with temporary.open("xb") as stream:
        stream.write(data)
    temporary.chmod(0o600)
    temporary.replace(path)


def selected(config, image) -> tuple[Receipt, Path]:
    path = receipt_path(config, image.section)
    data = producer_evidence.read_file(path, config.functional.native_images.receipt_max_bytes)
    receipt = Receipt.model_validate_json(data)
    if receipt.source != source(config.root) or receipt.platform != host_platform():
        raise ValueError("native candidate source/platform differs")
    layout = image.stage() / receipt.layout_key
    contained(image.stage(), layout)
    expected = identity(image, receipt.subject, receipt.platform, receipt.source)
    if receipt != expected or layout != image.layout_path(receipt.subject):
        raise ValueError("native candidate identity differs from original producer")
    return receipt, layout
