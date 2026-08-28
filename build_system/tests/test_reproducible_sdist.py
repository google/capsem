from __future__ import annotations

import gzip
import io
import tarfile
import tomllib
from pathlib import Path

from build_system.sdist_command import normalize_sdist

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _write_sdist(path: Path, *, build_time: int, uid: int, owner: str) -> None:
    payload = b"same source bytes\n"
    with (
        path.open("wb") as compressed,
        gzip.GzipFile(
            filename=path.name,
            mode="wb",
            fileobj=compressed,
            mtime=build_time,
        ) as gzip_file,
        tarfile.open(fileobj=gzip_file, mode="w", format=tarfile.PAX_FORMAT) as archive,
    ):
        directory = tarfile.TarInfo("capsem_builder-0.6.2")
        directory.type = tarfile.DIRTYPE
        directory.mode = 0o755
        directory.uid = uid
        directory.gid = uid
        directory.uname = owner
        directory.gname = owner
        directory.mtime = build_time + 0.25
        archive.addfile(directory)

        source = tarfile.TarInfo("capsem_builder-0.6.2/builder/__init__.py")
        source.mode = 0o644
        source.uid = uid
        source.gid = uid
        source.uname = owner
        source.gname = owner
        source.mtime = build_time + 0.5
        source.size = len(payload)
        archive.addfile(source, io.BytesIO(payload))


def test_sdist_normalization_is_byte_stable_across_build_metadata(tmp_path: Path) -> None:
    first = tmp_path / "first.tar.gz"
    second = tmp_path / "second.tar.gz"
    _write_sdist(first, build_time=1_700_000_001, uid=501, owner="local")
    _write_sdist(second, build_time=1_800_000_002, uid=1000, owner="runner")

    normalize_sdist(first, epoch=1_600_000_000)
    normalize_sdist(second, epoch=1_600_000_000)

    assert first.read_bytes() == second.read_bytes()
    with tarfile.open(first, "r:gz") as archive:
        members = archive.getmembers()
        assert [member.name for member in members] == sorted(member.name for member in members)
        assert {member.mtime for member in members} == {1_600_000_000}
        assert {member.uid for member in members} == {0}
        assert {member.gid for member in members} == {0}
        assert {member.uname for member in members} == {""}
        assert {member.gname for member in members} == {""}
        source = archive.extractfile("capsem_builder-0.6.2/builder/__init__.py")
        assert source is not None
        assert source.read() == b"same source bytes\n"


def test_setuptools_uses_the_reproducible_sdist_command() -> None:
    project = tomllib.loads((PROJECT_ROOT / "pyproject.toml").read_text())

    assert project["tool"]["setuptools"]["cmdclass"] == {
        "sdist": "sdist_command.ReproducibleSdist"
    }
