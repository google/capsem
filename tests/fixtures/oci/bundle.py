"""Guest-only OCI fixture; uses the profile's Python and libraries, never a registry."""

import contextlib
import hashlib
import json
import re
import shutil
import subprocess
import sys
import sysconfig
import tempfile
import time
from pathlib import Path


def command(*args, **kwargs):
    return subprocess.run(
        args, capture_output=True, text=True, timeout=15, check=False, **kwargs
    )


def wait_for(predicate, description, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError(f"timed out waiting for {description}")


class Bundle:
    def __init__(self):
        self.directory = Path(tempfile.mkdtemp(prefix="oci-", dir="/var/tmp"))
        self.rootfs = self.directory / "rootfs"
        self.scratch = self.directory / "scratch"
        self.state = self.directory / "state"
        self.id = self.directory.name
        self.cgroup = Path("/sys/fs/cgroup") / self.id
        self.scratch.mkdir(mode=0o777)
        self.scratch.chmod(0o777)
        self.rootfs.mkdir()
        self.directory.chmod(0o755)
        for name in ("proc", "dev", "scratch", "tmp"):
            (self.rootfs / name).mkdir()
        stdlib = Path(sysconfig.get_path("stdlib"))
        shutil.copytree(
            stdlib,
            self.rootfs / stdlib.relative_to("/"),
            ignore=shutil.ignore_patterns("__pycache__", "test", "tests"),
        )
        self.python = str(Path(sys.executable).resolve())
        self._copy(Path(self.python))
        libraries = [Path(self.python), *self.rootfs.rglob("*.so")]
        for library in libraries:
            result = command("ldd", str(library))
            assert result.returncode == 0, result.stderr
            for path in re.findall(r"(?:=>\s+)?(/[^\s()]+)", result.stdout):
                self._copy(Path(path))
        readonly = self.rootfs / "readonly"
        readonly.write_text("original")
        readonly.chmod(0o666)
        manifest = []
        for path in sorted(self.rootfs.rglob("*")):
            if path.is_file():
                manifest.append(
                    [
                        str(path.relative_to(self.rootfs)),
                        path.stat().st_mode & 0o777,
                        hashlib.sha256(path.read_bytes()).hexdigest(),
                    ]
                )
        self.digest = hashlib.sha256(json.dumps(manifest).encode()).hexdigest()
        Path("/root/oci/manifest.json").write_text(json.dumps(manifest))

    def _copy(self, source):
        target = self.rootfs / source.relative_to("/")
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            target.chmod(source.stat().st_mode & 0o777)

    def configure(self, source):
        config = {
            "ociVersion": "1.0.2",
            "root": {"path": "rootfs", "readonly": True},
            "hostname": "oci-fixture",
            "process": {
                "terminal": False,
                "user": {"uid": 65534, "gid": 65534},
                "args": [self.python, "-B", "-S", "-c", source],
                "cwd": "/scratch",
                "env": ["PATH=/usr/bin:/bin", "PYTHONHOME=/usr"],
                "noNewPrivileges": True,
                "capabilities": {
                    key: []
                    for key in (
                        "bounding",
                        "effective",
                        "inheritable",
                        "permitted",
                        "ambient",
                    )
                },
            },
            "mounts": [
                {
                    "destination": "/proc",
                    "type": "proc",
                    "source": "proc",
                    "options": ["nosuid", "nodev", "noexec"],
                },
                {
                    "destination": "/dev",
                    "type": "tmpfs",
                    "source": "tmpfs",
                    "options": ["nosuid", "noexec", "mode=755", "size=1m"],
                },
                {
                    "destination": "/scratch",
                    "type": "bind",
                    "source": str(self.scratch),
                    "options": ["bind", "rw", "nosuid", "nodev", "noexec"],
                },
            ],
            "linux": {
                "namespaces": [
                    {"type": kind}
                    for kind in ("mount", "pid", "uts", "ipc", "network", "cgroup")
                ],
                "cgroupsPath": f"/{self.id}",
                "resources": {
                    "memory": {"limit": 64 * 1024 * 1024, "swap": 64 * 1024 * 1024},
                    "pids": {"limit": 16},
                    "cpu": {"quota": 10000, "period": 100000},
                },
                # No device-deny BPF program: the guest keeps BPF_SYSCALL=n.
                # Non-root, empty caps, private /dev and nodev scratch prevent
                # acquisition of guest devices; the adversarial probe checks it.
                "seccomp": {
                    "defaultAction": "SCMP_ACT_ALLOW",
                    "syscalls": [
                        {
                            "names": ["socket"],
                            "action": "SCMP_ACT_ERRNO",
                            "errnoRet": 1,
                            "args": [{"index": 0, "value": 40, "op": "SCMP_CMP_EQ"}],
                        }
                    ],
                },
            },
        }
        payload = json.dumps(config)
        (self.directory / "config.json").write_text(payload)
        print(
            "OCI_CONFIG_SHA256=" + hashlib.sha256(payload.encode()).hexdigest(),
            flush=True,
        )

    def runc(self, *args):
        # This selects runc's rootless *cgroup manager*, not a rootless host
        # process or a user namespace. It tolerates unavailable device BPF;
        # all requested CPU/memory/pids limits must still succeed.
        return command("runc", "--rootless=true", "--root", str(self.state), *args)

    def assert_clean(self):
        result = self.runc("list", "--format", "json")
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout) in (None, []), result.stdout
        assert not self.cgroup.exists(), self.cgroup
        assert not any(
            str(self.rootfs) in line or str(self.scratch) in line
            for line in Path("/proc/self/mountinfo").read_text().splitlines()
        )

    @contextlib.contextmanager
    def running(self, source):
        self.configure(source)
        with (
            (self.directory / "stdout").open("w+") as stdout,
            (self.directory / "stderr").open("w+") as stderr,
        ):
            process = subprocess.Popen(
                [
                    "runc",
                    "--rootless=true",
                    "--root",
                    str(self.state),
                    "run",
                    "--no-new-keyring",
                    "--bundle",
                    str(self.directory),
                    self.id,
                ],
                stdout=stdout,
                stderr=stderr,
            )
            try:
                yield process
            except BaseException as error:
                error.add_note(f"runc output: {self.output()!r}")
                raise
            finally:
                listing = self.runc("list", "--format", "json")
                assert listing.returncode == 0, listing.stderr
                if json.loads(listing.stdout):
                    deleted = self.runc("delete", "--force", self.id)
                    assert deleted.returncode == 0, deleted.stderr
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                    raise
                self.assert_clean()

    def output(self):
        return (
            (self.directory / "stdout").read_text(),
            (self.directory / "stderr").read_text(),
        )

    def close(self):
        self.assert_clean()
        shutil.rmtree(self.directory)
