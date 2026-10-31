"""Prepare a real Linux supervisor, then qualify it without privilege or egress."""

from __future__ import annotations

import argparse
import errno
import os
import signal
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

from capsem_builder.gate import config as gate_config

from .systemd_manager import manager_environment, manager_pid


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--uid", type=int)
    parser.add_argument("--gid", type=int)
    parser.add_argument("--manager-runtime", type=Path)
    parser.add_argument("--manager-pid", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a qualification command is required")
    if args.inside:
        assert (os.getuid(), os.getgid()) == (args.uid, args.gid), "native supervisor UID/GID changed"
        assert args.uid != 0, "native SDK acceptance requires the ordinary user"
        assert "NoNewPrivs:\t1" in Path("/proc/self/status").read_text()
        assert args.manager_runtime is not None, "native supervisor manager runtime is required"
        assert manager_pid(args.manager_runtime, same_network=True) == args.manager_pid, (
            "native supervisor manager PID changed"
        )
        # Privilege/session tools can rewrite the ambient manager environment.
        # Bind only the manager we created and authenticated before entering;
        # never let an inherited host-user bus redirect the service outside it.
        os.environ["XDG_RUNTIME_DIR"] = str(args.manager_runtime)
        os.environ["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={args.manager_runtime / 'bus'}"
        assert [name for _, name in socket.if_nameindex()] == ["lo"]
        policy = gate_config.load(Path(__file__).resolve().parents[2]).sandbox
        with socket.socket() as probe:
            probe.settimeout(policy.linux_probe_timeout_seconds)
            try:
                probe.connect((policy.linux_probe_egress_host, policy.linux_probe_egress_port))
            except OSError as error:
                assert error.errno in {errno.ENETUNREACH, errno.EHOSTUNREACH}, error
            else:
                raise AssertionError("native supervisor worker has external network access")
        print("SDK_SUPERVISOR_KERNEL_PROOF_OK", flush=True)
        return subprocess.run(command, check=False).returncode

    previous = signal.getsignal(signal.SIGTERM)

    def interrupted(_signal: int, _frame: object) -> None:
        raise SystemExit("native supervisor qualification interrupted")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        caller_environment = dict(os.environ)
        with (
            tempfile.TemporaryDirectory(prefix="capsem-sdk-manager-") as directory,
            manager_environment(Path(directory), Path(directory).name, lambda: None, private_network=True) as manager,
        ):
            # Session hooks must see the caller's bus. The authenticated private
            # manager is rebound by --inside after sudo/PAM have completed.
            # Preserve names, never serialize or render inherited values.
            preserve = ",".join(sorted(caller_environment))
            groups = os.getgroups()
            group_flag = f"--groups={','.join(map(str, groups))}" if groups else "--clear-groups"
            assert __spec__ is not None, "run the supervisor owner as a Python module"
            argv = [
                "sudo", "-n", f"--preserve-env={preserve}",
                "/usr/bin/nsenter", f"--net=/proc/{manager.pid}/ns/net", "--",
                "/usr/bin/setpriv", f"--reuid={os.getuid()}", f"--regid={os.getgid()}",
                group_flag, "--no-new-privs", "--",
                sys.executable, "-m", __spec__.name, "--inside",
                "--uid", str(os.getuid()), "--gid", str(os.getgid()),
                "--manager-runtime", manager.environment["XDG_RUNTIME_DIR"],
                "--manager-pid", str(manager.pid), "--", *command,
            ]
            return subprocess.run(argv, env=caller_environment, check=False).returncode
    finally:
        signal.signal(signal.SIGTERM, previous)


if __name__ == "__main__":
    raise SystemExit(main())
