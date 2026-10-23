"""One immutable-image policy aliases AGY scratch to the existing workspace.

Runs in the guest VM's launcher namespace, never on the host. Originals and
credential siblings stay untouched. All guest-controlled traversal uses held
directory descriptors, no-follow opens and exclusive destination creation.
"""

import contextlib
import errno
import os
import stat
from pathlib import PurePosixPath

POLICY = "antigravity-scratch-v1"
LABEL = "org.capsem.workspace.policy"
HOME = "/home/capsem"
SCRATCH = ".gemini/antigravity/scratch"
DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK


def safe_mount_point(value):
    """An absolute normalized path outside the runtime's system mounts."""
    path = PurePosixPath(value)
    return (
        isinstance(value, str)
        and path.is_absolute()
        and ".." not in path.parts
        and str(path) == value
        and len(path.parts) >= 2
        and len(value) <= 4096
        and path.parts[1] not in {"proc", "dev", "sys", "usr", "bin", "sbin", "lib", "lib64", "etc"}
    )


def enabled(image, workspace):
    metadata = image.get("config") or {}
    policy = (metadata.get("Labels") or {}).get(LABEL)
    if policy is None:
        return False
    if (
        policy != POLICY
        or workspace != "/workspace"
        or HOME not in (metadata.get("Volumes") or {})
    ):
        raise ValueError("unsupported image workspace policy")
    return True


def alias_mounts(image, workspace, source, stage):
    if not enabled(image, workspace):
        return []
    destination = f"{HOME}/{SCRATCH}"
    return [
        {
            "destination": destination,
            "type": "bind",
            "source": source,
            "options": ["bind", "rw", "nosuid", "nodev"],
        },
        {
            "destination": f"{destination}/{stage}",
            "type": "tmpfs",
            "source": "tmpfs",
            "options": ["ro", "nosuid", "nodev", "noexec", "mode=000", "size=4k"],
        },
    ]


@contextlib.contextmanager
def directory(parent, name, owner=None, create=False):
    created = False
    try:
        fd = os.open(name, DIR_FLAGS, dir_fd=parent)
    except FileNotFoundError:
        if not create:
            raise
        os.mkdir(name, 0o700, dir_fd=parent)
        created = True
        fd = os.open(name, DIR_FLAGS, dir_fd=parent)
    try:
        if created and owner is not None:
            os.fchown(fd, *owner)
        yield fd
    finally:
        os.close(fd)


class Budget:
    def __init__(self, maximum):
        self.bytes = maximum
        self.entries = 16384

    def consume(self, count):
        self.bytes -= count
        if self.bytes < 0:
            raise ValueError("scratch import byte budget exceeded")


def same_file(source, destination, budget):
    if not stat.S_ISREG(os.fstat(destination).st_mode):
        return False
    if os.fstat(source).st_size != os.fstat(destination).st_size:
        return False
    os.lseek(source, 0, os.SEEK_SET)
    while True:
        left = os.read(source, 65536)
        right = os.read(destination, 65536)
        budget.consume(len(left) + len(right))
        if left != right:
            return False
        if not left:
            return True


def copy_file(source, destination, name, budget, owner):
    size = os.fstat(source).st_size
    if size > 64 * 1024 * 1024 or size > budget.bytes:
        raise ValueError("scratch import file budget exceeded")
    for suffix in range(128):
        candidate = name if suffix == 0 else f"{name}.imported-{suffix}"
        try:
            existing = os.open(candidate, FILE_FLAGS, dir_fd=destination)
        except FileNotFoundError:
            existing = None
        except OSError as error:
            if error.errno not in (errno.ELOOP, errno.ENOTDIR):
                raise
            continue
        if existing is not None:
            try:
                if same_file(source, existing, budget):
                    return
            finally:
                os.close(existing)
            continue
        try:
            output = os.open(
                candidate,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                0o600,
                dir_fd=destination,
            )
        except FileExistsError:
            continue
        try:
            if owner is not None:
                os.fchown(output, *owner)
            os.lseek(source, 0, os.SEEK_SET)
            while chunk := os.read(source, 65536):
                budget.consume(len(chunk))
                view = memoryview(chunk)
                while view:
                    count = os.write(output, view)
                    if count == 0:
                        raise OSError("scratch import made no write progress")
                    view = view[count:]
            os.fsync(output)
        finally:
            # Even on failure, never unlink or replace a concurrent guest entry.
            # The original remains intact; an incomplete import has no receipt.
            os.close(output)
        return
    raise ValueError("scratch import conflict budget exceeded")


def bounded_names(source, budget):
    names = []
    with os.scandir(source) as entries:
        for entry in entries:
            budget.entries -= 1
            if budget.entries < 0:
                raise ValueError("scratch import entry budget exceeded")
            names.append(entry.name)
    return sorted(names)


def copy_tree(source, destination, budget, owner, depth=0):
    if depth > 32:
        raise ValueError("scratch import depth budget exceeded")
    for name in bounded_names(source, budget):
        if name == ".capsem-image":
            raise ValueError("scratch import cannot name the launcher's stage")
        fd = os.open(name, FILE_FLAGS, dir_fd=source)
        try:
            mode = os.fstat(fd).st_mode
            if stat.S_ISREG(mode):
                copy_file(fd, destination, name, budget, owner)
            elif stat.S_ISDIR(mode):
                for suffix in range(128):
                    candidate = name if suffix == 0 else f"{name}.imported-{suffix}"
                    try:
                        with directory(
                            destination, candidate, owner, create=True
                        ) as target:
                            copy_tree(fd, target, budget, owner, depth + 1)
                        break
                    except OSError as error:
                        if error.errno not in (errno.ELOOP, errno.ENOTDIR):
                            raise
                else:
                    raise ValueError(
                        "scratch import directory conflict budget exceeded"
                    )
            else:
                raise ValueError("scratch import refuses non-regular entries")
        finally:
            os.close(fd)
    os.fsync(destination)


def copy_as_owner(source, destination, maximum, owner):
    if owner is None:
        copy_tree(source, destination, Budget(maximum), None)
        return
    if os.geteuid() != 0:
        if owner != (os.geteuid(), os.getegid()):
            raise ValueError("scratch import requires the image user")
        copy_tree(source, destination, Budget(maximum), None)
        return
    # Use the actual workload caller through the existing idmapped view.
    # Root-created files can have different owners on caller-owned VirtioFS.
    child = os.fork()
    if child == 0:
        status = 1
        try:
            os.setgroups([])
            os.setgid(owner[1])
            os.setuid(owner[0])
            copy_tree(source, destination, Budget(maximum), None)
            status = 0
        finally:
            os._exit(status)
    _, status = os.waitpid(child, 0)
    if status != 0:
        raise ValueError("scratch import failed as the image user")


def import_scratch(home, workspace, receipts, owner=None, max_bytes=256 * 1024 * 1024):
    with directory(None, receipts) as state:
        try:
            marker = os.open(POLICY, FILE_FLAGS, dir_fd=state)
        except FileNotFoundError:
            marker = None
        if marker is not None:
            try:
                if (
                    not stat.S_ISREG(os.fstat(marker).st_mode)
                    or os.read(marker, 4) != b"v1\n"
                ):
                    raise ValueError("invalid scratch import receipt")
                return
            finally:
                os.close(marker)
        with directory(None, home) as root, contextlib.ExitStack() as stack:
            source = root
            for name in SCRATCH.split("/"):
                source = stack.enter_context(
                    directory(source, name, owner, create=True)
                )
            with directory(None, workspace) as destination:
                copy_as_owner(source, destination, max_bytes, owner)
        marker = os.open(
            POLICY,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
            0o600,
            dir_fd=state,
        )
        try:
            os.write(marker, b"v1\n")
            os.fsync(marker)
        finally:
            os.close(marker)
