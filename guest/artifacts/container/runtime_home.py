"""Back workload /home with the already observed VirtioFS file tree.

One-time descriptor-only import preserves old image/volume state; subsequent
launches use the shared tree directly. Originals are never removed/replaced.
"""
import contextlib
import os
import runpy
import stat
from pathlib import Path

_files = runpy.run_path(str(Path(__file__).with_name("runtime_workspace.py")))
_userns = runpy.run_path(str(Path(__file__).with_name("runtime_userns.py")))
RECEIPT = "home-share-v1"


def mounts(workspace, source, stage):
    if workspace is None:
        return []
    return [
        {"destination": "/home", "type": "bind", "source": str(source),
         "options": ["bind", "rw", "nosuid", "nodev"]},
        {"destination": f"/home/{stage}", "type": "tmpfs", "source": "tmpfs",
         "options": ["ro", "nosuid", "nodev", "noexec", "mode=000", "size=4k"]},
    ]


def sources(rootfs, volumes, volume_dir):
    # More specific volumes win over their image/parent seed. Copying never
    # overwrites an existing file, including a workspace populated by a user.
    result = [(tuple(path.removeprefix("/home").strip("/").split("/")) if path != "/home" else (), volume_dir(path))
              for path in sorted(volumes, key=lambda p: len(Path(p).parts), reverse=True)
              if path == "/home" or path.startswith("/home/")]
    seed = rootfs / "home"
    if seed.is_dir() and not seed.is_symlink():
        result.append(((), seed))
    return result


def prepare(roots, workspace, receipts, owner, id_map, maximum=1024 * 1024 * 1024):
    directory = _files["directory"]
    flags = _files["FILE_FLAGS"]
    with directory(None, receipts) as state:
        try:
            marker = os.open(RECEIPT, flags, dir_fd=state)
        except FileNotFoundError:
            marker = None
        if marker is not None:
            try:
                if not stat.S_ISREG(os.fstat(marker).st_mode) or os.read(marker, 4) != b"v1\n":
                    raise ValueError("invalid home import receipt")
                return
            finally:
                os.close(marker)
        with contextlib.ExitStack() as stack:
            destination = stack.enter_context(directory(None, workspace))
            opened = [(parts, stack.enter_context(directory(None, source))) for parts, source in roots]

            def copy():
                os.fstat(destination)  # Refresh VirtioFS caller-relative metadata.
                budget = _files["Budget"](maximum, preserve=True)
                for parts, source in opened:
                    with contextlib.ExitStack() as parents:
                        target = destination
                        for part in parts:
                            target = parents.enter_context(directory(target, part, create=True))
                        _files["copy_tree"](source, target, budget, None)

            if id_map is None:
                copy()
            else:
                _userns["run_as_user"](id_map, owner, copy)
        marker = os.open(RECEIPT, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=state)
        try:
            os.write(marker, b"v1\n")
            os.fsync(marker)
        finally:
            os.close(marker)
