"""Adversarial guest tests for the packaged OCI runtime, invoked by Ironbank."""

import json
import signal
import subprocess
import unittest
from pathlib import Path

from bundle import Bundle, wait_for

HOLD = "\nimport time\ntime.sleep(60)\n"


class OfflineContainer(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bundle = Bundle()
        print("OCI_ROOTFS_SHA256=" + cls.bundle.digest, flush=True)

    @classmethod
    def tearDownClass(cls):
        cls.bundle.close()

    def setUp(self):
        for path in self.bundle.scratch.iterdir():
            path.unlink()

    def test_output_exit_and_binary_file(self):
        (self.bundle.scratch / "input").write_bytes(
            Path("/root/oci/input.bin").read_bytes()
        )
        with self.bundle.running(
            "import os,sys; assert os.getpid()==1; assert os.getuid()==65534; "
            "open('/scratch/bytes','wb').write(open('/scratch/input','rb').read()); "
            "print('oci-out',flush=True); print('oci-err',file=sys.stderr,flush=True); "
            "sys.exit(23)"
        ) as process:
            self.assertEqual(process.wait(timeout=15), 23, self.bundle.output())
        self.assertEqual(self.bundle.output(), ("oci-out\n", "oci-err\n"))
        self.assertEqual(
            (self.bundle.scratch / "bytes").read_bytes(), bytes(range(256))
        )
        Path("/root/oci/output.bin").write_bytes(
            (self.bundle.scratch / "bytes").read_bytes()
        )

    def test_files_devices_and_network_are_isolated(self):
        secret = self.bundle.directory / "guest-secret"
        secret.write_text("not-mounted")
        # Writable mode makes the EROFS check distinguish a read-only mount
        # from an ordinary DAC denial for our non-root workload.
        target = self.bundle.rootfs / "readonly"
        source = f"""
import ctypes, errno, os, socket, stat
libc = ctypes.CDLL(None, use_errno=True)
bpf_syscall = {{'aarch64': 280, 'x86_64': 321}}[os.uname().machine]
assert libc.syscall(bpf_syscall, 0, 0, 0) == -1 and ctypes.get_errno() == errno.ENOSYS
assert not os.path.exists({str(secret)!r})
assert not os.path.exists('/dev/vda')
assert not os.path.exists('/sys/fs/cgroup')
assert os.readlink('/proc/self/ns/net') != {str(Path("/proc/self/ns/net").readlink())!r}
assert 'CapEff:\\t0000000000000000' in open('/proc/self/status').read()
try:
    open('/readonly', 'w')
except OSError as error:
    assert error.errno == errno.EROFS, error
else:
    raise AssertionError('rootfs writable')
try:
    os.mknod('/scratch/disk', stat.S_IFBLK | 0o600, os.makedev(254, 0))
except PermissionError:
    pass
else:
    raise AssertionError('device creation permitted')
for family, address in [(socket.AF_INET, ('192.0.2.1', 443)), (40, (2, 5002))]:
    try:
        with socket.socket(family, socket.SOCK_STREAM) as connection:
            connection.settimeout(1)
            connection.connect(address)
    except OSError as error:
        assert error.errno == (errno.ENETUNREACH if family == socket.AF_INET else errno.EPERM), error
    else:
        raise AssertionError('network escape')
print('isolated')
"""
        with self.bundle.running(source) as process:
            self.assertEqual(process.wait(timeout=15), 0, self.bundle.output())
        self.assertEqual(self.bundle.output()[0], "isolated\n")
        self.assertEqual(secret.read_text(), "not-mounted")
        self.assertEqual(target.read_text(), "original")

    def test_process_limit(self):
        source = """
import errno, os, time
for attempt in range(32):
    try:
        child = os.fork()
    except OSError as error:
        assert error.errno == errno.EAGAIN, error
        open('/scratch/ready', 'w').write(str(attempt))
        break
    if child == 0:
        time.sleep(60)
        os._exit(0)
else:
    raise AssertionError('pids limit not enforced')
"""
        with self.bundle.running(source + HOLD):
            wait_for(lambda: (self.bundle.scratch / "ready").exists(), "pids limit")
            self.assertEqual(
                (self.bundle.cgroup / "pids.max").read_text().strip(), "16"
            )
            self.assertEqual(int((self.bundle.cgroup / "pids.current").read_text()), 16)
            self.assertEqual((self.bundle.scratch / "ready").read_text(), "15")

    def test_memory_limit(self):
        source = """
import os
if os.fork() == 0:
    chunks = [bytearray(1024 * 1024) for _ in range(128)]
    os._exit(42)
_, status = os.wait()
assert os.WIFSIGNALED(status) and os.WTERMSIG(status) == 9, status
open('/scratch/ready', 'w').write('oom')
"""
        with self.bundle.running(source + HOLD):
            wait_for(lambda: (self.bundle.scratch / "ready").exists(), "memory limit")
            self.assertEqual(
                int((self.bundle.cgroup / "memory.max").read_text()), 67108864
            )
            events = dict(
                line.split()
                for line in (self.bundle.cgroup / "memory.events")
                .read_text()
                .splitlines()
            )
            self.assertGreaterEqual(int(events["oom_kill"]), 1)
            self.assertLessEqual(
                int((self.bundle.cgroup / "memory.current").read_text()), 67108864
            )

    def test_cpu_quota(self):
        source = """
import time
open('/scratch/ready', 'w').write('cpu')
deadline = time.monotonic() + 3
while time.monotonic() < deadline:
    pass
"""
        with self.bundle.running(source + HOLD):
            wait_for(lambda: (self.bundle.scratch / "ready").exists(), "CPU workload")
            self.assertEqual(
                (self.bundle.cgroup / "cpu.max").read_text().strip(), "10000 100000"
            )

            def throttled():
                stats = dict(
                    line.split()
                    for line in (self.bundle.cgroup / "cpu.stat")
                    .read_text()
                    .splitlines()
                )
                return int(stats["nr_throttled"]) >= 3 and int(stats["usage_usec"]) > 0

            wait_for(throttled, "observable CPU throttling")

    def test_cancellation_kills_descendants(self):
        source = """
import os, signal, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
child = os.fork()
if child:
    open('/scratch/ready', 'w').write(str(child))
while True:
    time.sleep(1)
"""
        with self.bundle.running(source) as process:
            wait_for(lambda: (self.bundle.scratch / "ready").exists(), "descendant")
            process.send_signal(signal.SIGTERM)
            # An uncooperative workload requires forced cancellation. Exercise
            # the actual OCI lifecycle, then verify cgroup and mount removal.
            result = self.bundle.runc("kill", "--all", self.bundle.id, "KILL")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(process.wait(timeout=10), 137, self.bundle.output())
        self.bundle.assert_clean()

    def test_deadline_forces_cleanup(self):
        source = (
            """
import os, signal
signal.signal(signal.SIGTERM, signal.SIG_IGN)
if os.fork():
    open('/scratch/ready', 'w').write('descendant')
"""
            + HOLD
        )
        with self.bundle.running(source) as process:
            wait_for(
                lambda: (self.bundle.scratch / "ready").exists(), "live descendant"
            )
            with self.assertRaises(subprocess.TimeoutExpired):
                process.wait(timeout=0.5)
        # running() owns forced deletion after the deadline, including cases
        # where the caller never reaches a normal runc wait/exit path.
        self.bundle.assert_clean()

    def test_failed_launch_leaves_no_state(self):
        with self.bundle.running("raise RuntimeError('fixture failure')") as process:
            self.assertEqual(process.wait(timeout=15), 1, self.bundle.output())
        self.assertIn("fixture failure", self.bundle.output()[1])
        # Fail during runtime setup, before any container process can start.
        self.bundle.configure("pass")
        config_path = self.bundle.directory / "config.json"
        config = json.loads(config_path.read_text())
        config["root"]["path"] = "missing-rootfs"
        config_path.write_text(json.dumps(config))
        result = self.bundle.runc(
            "run",
            "--no-new-keyring",
            "--bundle",
            str(self.bundle.directory),
            self.bundle.id,
        )
        self.assertNotEqual(result.returncode, 0)
        self.bundle.assert_clean()


if __name__ == "__main__":
    unittest.main(verbosity=2)
