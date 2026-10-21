"""Own runc's actual workload terminal and reap its detached workload.

runc's detached console protocol transfers the PTY master over SCM_RIGHTS.
There is no outer terminal whose modes can buffer keys or translate output.
This module runs only in the dedicated Linux guest launcher.
"""

import array
import ctypes
import os
import socket
import subprocess
import time


def receive_terminal(connection):
    """Accept exactly one non-inheritable terminal FD; close every refused FD."""
    descriptors = array.array("i")
    # Linux permits at most 253 SCM_RIGHTS FDs. Receive all of that bounded
    # message so even an invalid multi-FD handoff can be closed completely.
    data, ancillary, flags, _ = connection.recvmsg(4096, socket.CMSG_SPACE(256 * descriptors.itemsize), getattr(socket, "MSG_CMSG_CLOEXEC", 0))
    valid = bool(data) and not flags & (socket.MSG_CTRUNC | socket.MSG_TRUNC)
    for level, kind, value in ancillary:
        if (level, kind) != (socket.SOL_SOCKET, socket.SCM_RIGHTS):
            valid = False
            continue
        complete = len(value) - len(value) % descriptors.itemsize
        descriptors.frombytes(value[:complete])
        valid &= complete == len(value)
    try:
        if not valid or len(descriptors) != 1 or not os.isatty(descriptors[0]):
            raise ValueError("runc must hand off exactly one terminal descriptor")
        os.set_inheritable(descriptors[0], False)
        return descriptors.pop()
    finally:
        for fd in descriptors:
            os.close(fd)


def become_subreaper():
    # PR_SET_CHILD_SUBREAPER: runc exits after handoff; the workload and its
    # descendants must become this launcher's children, not orphaned daemons.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


class RuntimeConsole:
    def __init__(self):
        self.master = None
        self.pid = None
        self.returncode = None

    def start(self, argv, container, path, pid_file, timeout):
        become_subreaper()
        deadline = time.monotonic() + timeout
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(path))
            path.chmod(0o600)
            listener.listen(1)
            listener.settimeout(min(0.05, timeout))
            runtime = subprocess.Popen(
                [*argv, "--detach", "--console-socket", str(path), container], stdin=subprocess.DEVNULL
            )
            try:
                while True:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("runc terminal handoff timed out")
                    try:
                        connection, _ = listener.accept()
                        break
                    except TimeoutError:
                        if runtime.poll() is not None:
                            raise RuntimeError(f"runc exited before terminal handoff: {runtime.returncode}")
                with connection:
                    connection.settimeout(max(0.001, deadline - time.monotonic()))
                    self.master = receive_terminal(connection)
                result = runtime.wait(timeout=max(0.001, deadline - time.monotonic()))
                if result != 0:
                    raise RuntimeError(f"runc failed after terminal handoff: {result}")
                pid = int(pid_file.read_text())
                if pid <= 1:
                    raise ValueError("runc supplied an invalid workload pid")
                self.pid = pid
                return self.master
            finally:
                if runtime.poll() is None:
                    runtime.kill()
                runtime.wait(timeout=5)
                path.unlink()

    def wait(self, timeout=None):
        """Reap all adopted children, retaining the workload's actual exit code.

        Only the dedicated launcher calls this, after runc itself was reaped.
        PID-namespace teardown ends the workload's remaining descendants.
        """
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            try:
                pid, status = os.waitpid(-1, 0 if deadline is None else os.WNOHANG)
            except ChildProcessError:
                if self.pid is not None and self.returncode is None:
                    raise RuntimeError("runc workload was not adopted by its launcher") from None
                return self.returncode
            if pid == self.pid:
                self.returncode = os.waitstatus_to_exitcode(status)
                if self.returncode < 0:
                    self.returncode = 128 - self.returncode
            if pid == 0:
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired("workload exit", timeout)
                time.sleep(min(0.01, max(0, deadline - time.monotonic())))

    def close(self):
        if self.master is not None:
            os.close(self.master)
            self.master = None
