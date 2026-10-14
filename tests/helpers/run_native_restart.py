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

from .systemd_manager import manager_environment


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--uid", type=int)
    parser.add_argument("--gid", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a qualification command is required")
    if args.inside:
        assert (os.getuid(), os.getgid()) == (args.uid, args.gid)
        assert args.uid != 0, "native SDK acceptance requires the ordinary user"
        assert "NoNewPrivs:\t1" in Path("/proc/self/status").read_text()
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
        with (
            tempfile.TemporaryDirectory(prefix="capsem-sdk-manager-") as directory,
            manager_environment(Path(directory), Path(directory).name, lambda: None, private_network=True) as manager,
        ):
            # Preserve names, never serialize or render inherited values.
            preserve = ",".join(sorted(manager.environment))
            groups = os.getgroups()
            group_flag = f"--groups={','.join(map(str, groups))}" if groups else "--clear-groups"
            assert __spec__ is not None, "run the supervisor owner as a Python module"
            argv = [
                "sudo", "-n", f"--preserve-env={preserve}",
                "/usr/bin/nsenter", f"--net=/proc/{manager.pid}/ns/net", "--",
                "/usr/bin/setpriv", f"--reuid={os.getuid()}", f"--regid={os.getgid()}",
                group_flag, "--no-new-privs", "--",
                sys.executable, "-m", __spec__.name, "--inside",
                "--uid", str(os.getuid()), "--gid", str(os.getgid()), "--", *command,
            ]
            return subprocess.run(argv, env=manager.environment, check=False).returncode
    finally:
        signal.signal(signal.SIGTERM, previous)


if __name__ == "__main__":
    raise SystemExit(main())
