"""Setuptools adapter for reproducible source distributions."""

from __future__ import annotations

import copy
import gzip
import io
import os
import tarfile
from pathlib import Path
from typing import TYPE_CHECKING

from setuptools.command.sdist import sdist as SetuptoolsSdist

if TYPE_CHECKING:
    from _typeshed import StrOrBytesPath, StrPath

_MAX_GZIP_EPOCH = (1 << 32) - 1


def _source_date_epoch() -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH")
    epoch = int(raw) if raw is not None else 0
    if not 0 <= epoch <= _MAX_GZIP_EPOCH:
        raise ValueError(f"SOURCE_DATE_EPOCH must be between 0 and {_MAX_GZIP_EPOCH}")
    return epoch


def normalize_sdist(path: Path, *, epoch: int) -> None:
    """Rewrite a gzip-compressed tar archive with stable metadata and order."""
    if not 0 <= epoch <= _MAX_GZIP_EPOCH:
        raise ValueError(f"epoch must be between 0 and {_MAX_GZIP_EPOCH}")

    entries: list[tuple[tarfile.TarInfo, bytes | None]] = []
    with tarfile.open(path, "r:gz") as source:
        for member in source.getmembers():
            payload = None
            if member.isfile():
                extracted = source.extractfile(member)
                if extracted is None:
                    raise ValueError(f"sdist member has no payload: {member.name}")
                payload = extracted.read()
            entries.append((copy.copy(member), payload))

    output = io.BytesIO()
    with (
        gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=epoch) as compressed,
        tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as target,
    ):
        for member, payload in sorted(entries, key=lambda entry: entry[0].name):
            member.uid = 0
            member.gid = 0
            member.uname = ""
            member.gname = ""
            member.mtime = epoch
            member.pax_headers = {}
            target.addfile(member, io.BytesIO(payload) if payload is not None else None)

    path.write_bytes(output.getvalue())


class ReproducibleSdist(SetuptoolsSdist):
    """Normalize setuptools' generated archive before exposing it to the frontend."""

    def make_archive(
        self,
        base_name: StrPath,
        format: str,
        root_dir: StrOrBytesPath | None = None,
        base_dir: str | None = None,
        owner: str | None = None,
        group: str | None = None,
    ) -> str:
        archive = super().make_archive(
            os.fspath(base_name), format, root_dir, base_dir, owner, group
        )
        if not self.dry_run and format == "gztar":
            normalize_sdist(Path(archive), epoch=_source_date_epoch())
        return archive
