#!/usr/bin/env python3
"""Prove that an installed Capsem can open and execute in a guest shell."""

from __future__ import annotations

import argparse
import fcntl
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
import uuid
from pathlib import Path


SAFE_VALUE = re.compile(r"^[A-Za-z0-9_.-]+$")


def guest_marker_command(marker: str, proof_name: str) -> bytes:
    """Build a command whose input bytes do not contain the success marker."""
    octal = "".join(f"\\{byte:03o}" for byte in marker.encode("utf-8"))
    return f"printf '{octal}\\n' | tee \"$HOME/{proof_name}\"\r".encode("ascii")


def guest_proof_paths(proof_name: str) -> list[Path]:
    """Return host VirtioFS paths matching the proof file written in guest $HOME."""
    run_dir = Path(
        os.environ.get("CAPSEM_RUN_DIR", str(Path.home() / ".capsem" / "run"))
    )
    persistent = run_dir / "persistent"
    if not persistent.is_dir():
        return []
    return list(persistent.glob(f"*/guest/workspace/{proof_name}"))


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except OSError:
        try:
            process.terminate()
        except OSError:
            pass
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            try:
                process.kill()
            except OSError:
                pass


def prove_shell(
    capsem: Path,
    marker: str,
    session_name: str,
    timeout: float,
    startup_delay: float,
    retry_interval: float,
) -> None:
    create = subprocess.run(
        [str(capsem), "create", "--name", session_name],
        check=False,
        text=True,
        capture_output=True,
        timeout=timeout,
    )
    if create.returncode != 0:
        raise RuntimeError(f"failed to create shell-proof session: {create.stdout}{create.stderr}")

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    process = subprocess.Popen(
        [str(capsem), "shell", "--name", session_name],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        start_new_session=True,
    )
    os.close(slave)

    marker_bytes = marker.encode("utf-8")
    proof_name = f".capsem-shell-proof-{uuid.uuid4().hex}"
    command = guest_marker_command(marker, proof_name)
    output = bytearray()
    deadline = time.monotonic() + timeout
    next_send = time.monotonic() + startup_delay
    observed = False

    try:
        while time.monotonic() < deadline:
            now = time.monotonic()
            if now >= next_send:
                os.write(master, command)
                next_send = now + retry_interval

            readable, _, _ = select.select([master], [], [], 0.2)
            if readable:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    chunk = b""
                if chunk:
                    output.extend(chunk)
                    sys.stdout.buffer.write(chunk)
                    sys.stdout.buffer.flush()
                    if marker_bytes in output:
                        observed = True
                        break
            if any(
                path.is_file() and path.read_bytes().rstrip(b"\r\n") == marker_bytes
                for path in guest_proof_paths(proof_name)
            ):
                observed = True
                break
            if process.poll() is not None:
                break

        if not observed:
            tail = bytes(output[-4000:]).decode("utf-8", errors="replace")
            raise RuntimeError(
                "guest shell marker was not observed before timeout; "
                f"terminal tail follows:\n{tail}"
            )

        # Exit the guest shell, then use the TUI's global Alt-Q shortcut.
        try:
            os.write(master, b"exit\r")
        except OSError:
            pass
        time.sleep(0.5)
        if process.poll() is None:
            try:
                os.write(master, b"\x1bq")
            except OSError:
                pass
    finally:
        os.close(master)
        stop_process(process)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capsem", type=Path, required=True)
    parser.add_argument("--marker", required=True)
    parser.add_argument("--session-name", default=f"installed-shell-proof-{os.getpid()}")
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--startup-delay", type=float, default=2.0)
    parser.add_argument("--retry-interval", type=float, default=5.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    for label, value in (("marker", args.marker), ("session name", args.session_name)):
        if not SAFE_VALUE.fullmatch(value):
            raise SystemExit(f"{label} contains unsupported characters: {value!r}")
    if args.timeout <= 0 or args.startup_delay < 0 or args.retry_interval <= 0:
        raise SystemExit("timeout values must be positive")
    if not args.capsem.is_file() or not os.access(args.capsem, os.X_OK):
        raise SystemExit(f"capsem executable not found: {args.capsem}")

    try:
        prove_shell(
            args.capsem,
            args.marker,
            args.session_name,
            args.timeout,
            args.startup_delay,
            args.retry_interval,
        )
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"installed shell proof failed: {error}", file=sys.stderr)
        return 1
    finally:
        try:
            subprocess.run(
                [str(args.capsem), "delete", args.session_name],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=min(args.timeout, 30),
            )
        except (OSError, subprocess.TimeoutExpired):
            pass

    print(f"installed shell proof passed: {args.marker}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
