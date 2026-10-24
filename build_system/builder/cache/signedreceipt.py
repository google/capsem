"""Private executable identity handoff from the existing signed-copy producer.

These are replacement-race stamps, not signature or content-hash authority.
The shell runner still owns signing and its native signature-cache receipts.
"""

from __future__ import annotations

import stat
import sys
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, StrictInt


class FileStamp(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)
    path: Path
    device: StrictInt
    inode: StrictInt
    mode: StrictInt
    size: StrictInt
    mtime_ns: StrictInt
    ctime_ns: StrictInt

    @classmethod
    def capture(cls, path: Path) -> FileStamp:
        if not path.is_absolute():
            raise ValueError("executable handoff requires absolute paths")
        observed = path.lstat()
        if not stat.S_ISREG(observed.st_mode):
            raise ValueError("executable handoff requires a regular non-symlink file")
        return cls(
            path=path,
            device=observed.st_dev,
            inode=observed.st_ino,
            mode=observed.st_mode,
            size=observed.st_size,
            mtime_ns=observed.st_mtime_ns,
            ctime_ns=observed.st_ctime_ns,
        )

    def verify(self) -> None:
        if self != self.capture(self.path):
            raise ValueError("executable or policy input was replaced during handoff")


class Snapshot(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)
    files: tuple[FileStamp, ...]

    def verify(self) -> None:
        for member in self.files:
            member.verify()


class PreparedExecutable(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)
    schema_version: Literal["capsem.signed-executable.v1"]
    original: FileStamp
    executable: FileStamp
    policy: tuple[FileStamp, FileStamp]
    argv0: str
    assets_dir: Path

    def verify(self, *, original: Path, policy: tuple[Path, Path], assets_dir: Path) -> None:
        if (
            self.original.path != original
            or tuple(row.path for row in self.policy) != policy
            or self.argv0 != str(original)
            or self.assets_dir != assets_dir
            or self.executable.path == original
        ):
            raise ValueError("signed executable receipt has foreign bindings")
        self.original.verify()
        self.executable.verify()
        for member in self.policy:
            member.verify()


def decode_snapshot(text: str, maximum: int) -> Snapshot:
    if len(text.encode()) > maximum:
        raise ValueError("signed executable receipt byte budget exceeded")
    return Snapshot.model_validate_json(text)


def main(argv: list[str]) -> None:
    action, limit, *args = argv
    maximum = int(limit)
    if maximum <= 0:
        raise ValueError("signed executable receipt budget must be positive")
    if action == "capture":
        document = Snapshot(files=tuple(FileStamp.capture(Path(path)) for path in args))
    elif action == "emit":
        original, executable, assets, before, signed = args
        source = decode_snapshot(before, maximum)
        copy = decode_snapshot(signed, maximum)
        if len(source.files) != 3 or len(copy.files) != 1:
            raise ValueError("signed executable receipt input coverage differs")
        source.verify()
        copy.verify()
        if source.files[0].path != Path(original) or copy.files[0].path != Path(executable):
            raise ValueError("signed executable receipt input identity differs")
        document = PreparedExecutable(
            schema_version="capsem.signed-executable.v1",
            original=source.files[0],
            executable=copy.files[0],
            policy=(source.files[1], source.files[2]),
            argv0=original,
            assets_dir=Path(assets),
        )
    else:
        raise ValueError("unknown signed executable receipt action")
    text = document.model_dump_json()
    if len(text.encode()) > maximum:
        raise ValueError("signed executable receipt byte budget exceeded")
    # No ambient environment, tokens or contents are included in this receipt.
    sys.stdout.write(text)


if __name__ == "__main__":
    main(sys.argv[1:])
