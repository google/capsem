"""Normalize binary staging inputs and write deterministic package fixtures."""

from __future__ import annotations

import argparse
import gzip
import io
import os
import tarfile
from pathlib import Path


def _normalize_mtimes(root: Path) -> None:
    if not root.is_dir():
        raise SystemExit(f"binary staging work root is not a directory: {root}")
    paths = sorted(root.rglob("*"), key=lambda path: len(path.parts), reverse=True)
    for path in [*paths, root]:
        os.utime(path, (0, 0), follow_symlinks=False)


def _tar_gzip(root: Path, paths: list[Path]) -> bytes:
    buffer = io.BytesIO()
    with (
        gzip.GzipFile(filename="", mode="wb", fileobj=buffer, mtime=0) as compressed,
        tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as archive,
    ):
        for path in paths:
            relative = path.relative_to(root).as_posix()
            info = archive.gettarinfo(str(path), arcname=relative)
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            info.mtime = 0
            if info.isdir():
                info.mode = 0o755
            elif info.isfile():
                info.mode = 0o755 if info.mode & 0o111 else 0o644
            if path.is_file():
                with path.open("rb") as source:
                    archive.addfile(info, source)
            else:
                archive.addfile(info)
    return buffer.getvalue()


def _sorted_paths(root: Path) -> list[Path]:
    return sorted(root.rglob("*"), key=lambda path: path.relative_to(root).as_posix())


def _ar_member(name: str, contents: bytes) -> bytes:
    identifier = f"{name}/"
    if len(identifier) > 16:
        raise SystemExit(f"binary staging ar member name is too long: {name}")
    header = (
        f"{identifier:<16}{0:<12}{0:<6}{0:<6}{0o100644:<8o}"
        f"{len(contents):<10}`\n"
    ).encode("ascii")
    return header + contents + (b"\n" if len(contents) % 2 else b"")


def _write_synthetic_deb(root: Path, output: Path) -> None:
    control_root = root / "DEBIAN"
    if not control_root.is_dir():
        raise SystemExit(f"binary staging control root is not a directory: {control_root}")
    if output.exists():
        raise SystemExit(f"refusing stale binary staging deb: {output}")

    data_paths = [
        path
        for path in _sorted_paths(root)
        if path.relative_to(root).parts[0] != "DEBIAN"
    ]
    contents = b"!<arch>\n" + b"".join(
        (
            _ar_member("debian-binary", b"2.0\n"),
            _ar_member("control.tar.gz", _tar_gzip(control_root, _sorted_paths(control_root))),
            _ar_member("data.tar.gz", _tar_gzip(root, data_paths)),
        )
    )
    with output.open("xb") as destination:
        destination.write(contents)


def _write_synthetic_pkg(root: Path, output: Path) -> None:
    if not root.is_dir():
        raise SystemExit(f"binary staging pkg root is not a directory: {root}")
    if output.exists():
        raise SystemExit(f"refusing stale binary staging package: {output}")

    with output.open("xb") as destination:
        destination.write(_tar_gzip(root, _sorted_paths(root)))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("work_root", type=Path)
    parser.add_argument("deb_root", type=Path)
    parser.add_argument("deb_output", type=Path)
    parser.add_argument("pkg_root", type=Path)
    parser.add_argument("pkg_output", type=Path)
    args = parser.parse_args()

    _normalize_mtimes(args.work_root)
    _write_synthetic_deb(args.deb_root, args.deb_output)
    _write_synthetic_pkg(args.pkg_root, args.pkg_output)


if __name__ == "__main__":
    main()
