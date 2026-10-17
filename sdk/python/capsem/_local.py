"""Read installed gateway metadata without starting services or running commands."""

import os
import stat
from pathlib import Path

_ERROR = "Cannot read local Capsem gateway metadata. Start Capsem and try again; check CAPSEM_RUN_DIR or CAPSEM_HOME if customized."


def _read(path: Path, *, private: bool = False) -> str:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise ValueError("not an owned regular file")
        if private and info.st_mode & 0o077:
            raise ValueError("token file is not private")
        data = stream.read(4097)
        if len(data) > 4096:
            raise ValueError("metadata is too large")
        return data.decode("ascii").strip()


def capsem_run_dir(run_dir: str | Path | None = None) -> Path:
    """Resolve the runtime directory without opening gateway metadata."""
    if run_dir is not None:
        raw = str(run_dir).strip()
        if not raw:
            raise ValueError("run_dir must be a nonempty path")
        return Path(raw).expanduser()
    root = os.environ.get("CAPSEM_RUN_DIR", "").strip()
    if root:
        return Path(root).expanduser()
    home = os.environ.get("CAPSEM_HOME", "").strip()
    return (Path(home).expanduser() if home else Path.home() / ".capsem") / "run"


def local_connection(run_dir: str | Path | None = None) -> tuple[str, str]:
    try:
        directory = capsem_run_dir(run_dir)
        token = _read(directory / "gateway.token", private=True)
        port = _read(directory / "gateway.port")
        if not port.isascii() or not port.isdecimal() or not 1 <= int(port) <= 65535:
            raise ValueError("invalid local port")
        if not token or any(not 33 <= ord(char) <= 126 for char in token):
            raise ValueError("invalid local token")
        return f"http://127.0.0.1:{int(port)}", token
    except (OSError, ValueError, RuntimeError):
        raise ConnectionError(_ERROR) from None
