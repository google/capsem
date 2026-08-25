#!/usr/bin/env python3
"""Prove that an installed Capsem can open and execute in a guest shell."""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
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

# Lifecycle states the service never resumes from. `Stopped`/`Suspended` are
# absent on purpose: those are terminal only when the service also refuses to
# resume them, which `can_resume` reports separately.
FATAL_SESSION_STATUSES = frozenset({"Defunct", "Incompatible"})

# How often to ask the service whether the session is still on its way to a
# prompt. Cheap next to the boot it is watching, and rare enough that a
# healthy proof spends its time reading the terminal.
SESSION_STATE_POLL_INTERVAL = 3.0


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


def guest_shell_ready(output: bytes, session_name: str) -> bool:
    """Return whether the focused TUI has rendered this guest's shell prompt."""
    return f"root@{session_name}:".encode("ascii") in output


def session_boot_failure(info: dict[str, object]) -> str | None:
    """Return why a session can no longer reach a guest shell, or None while it can.

    `capsem create` returns as soon as the VM process is launched, so a boot
    that dies afterwards -- a bad asset pin, an unbuildable VmConfig -- leaves
    the TUI parked on its non-resumable screen and no prompt ever arrives.
    Waiting for the marker in that state burns the whole timeout and reports
    nothing. The service already knows the reason: `last_error` carries the
    `process.log` tail of a defunct session, and `resume_blocked_reason`
    carries the validation failure of one it refuses to resume.
    """
    status = info.get("status")
    if not isinstance(status, str) or status == "Running":
        return None
    if status not in FATAL_SESSION_STATUSES and info.get("can_resume") is not False:
        return None
    for key in ("last_error", "resume_blocked_reason"):
        reason = info.get(key)
        if isinstance(reason, str) and reason.strip():
            return f"session is {status}: {reason.strip()}"
    return f"session is {status} and the service recorded no reason"


def session_state_failure(capsem: Path, session_name: str, timeout: float) -> str | None:
    """Ask the service whether the proof session has already lost its shell.

    Every failure to reach the service is reported as "still waiting": the
    marker stays the only proof of success, so a transient query error must
    never fail a proof that is otherwise on track.
    """
    try:
        info = subprocess.run(
            [str(capsem), "info", session_name, "--json"],
            check=False,
            text=True,
            capture_output=True,
            timeout=min(timeout, 30),
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if info.returncode != 0:
        return None
    try:
        parsed = json.loads(info.stdout)
    except json.JSONDecodeError:
        return None
    return session_boot_failure(parsed) if isinstance(parsed, dict) else None


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except OSError:
        with contextlib.suppress(OSError):
            process.terminate()
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            with contextlib.suppress(OSError):
                process.kill()


def prove_shell(
    capsem: Path,
    marker: str,
    session_name: str,
    profile: str | None,
    timeout: float,
    startup_delay: float,
) -> None:
    create_args = [str(capsem), "create", "--name", session_name]
    if profile is not None:
        create_args.extend(["--profile", profile])
    create = subprocess.run(
        create_args,
        check=False,
        text=True,
        capture_output=True,
        timeout=timeout,
    )
    if create.returncode != 0:
        raise RuntimeError(f"failed to create shell-proof session: {create.stdout}{create.stderr}")
    if profile is not None:
        info = subprocess.run(
            [str(capsem), "info", session_name, "--json"],
            check=False,
            text=True,
            capture_output=True,
            timeout=timeout,
        )
        if info.returncode != 0:
            raise RuntimeError(
                f"failed to inspect shell-proof session profile: {info.stdout}{info.stderr}"
            )
        try:
            actual_profile = json.loads(info.stdout)["profile_id"]
        except (json.JSONDecodeError, KeyError, TypeError) as error:
            raise RuntimeError(
                f"shell-proof session info did not identify its profile: {info.stdout}"
            ) from error
        if actual_profile != profile:
            raise RuntimeError(
                "shell-proof session profile mismatch: "
                f"requested {profile!r}, service reported {actual_profile!r}"
            )

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
    send_after = time.monotonic() + startup_delay
    poll_state_after = time.monotonic() + SESSION_STATE_POLL_INTERVAL
    command_sent = False
    observed = False
    boot_failure: str | None = None

    try:
        while time.monotonic() < deadline:
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
            if (
                not command_sent
                and time.monotonic() >= send_after
                and guest_shell_ready(output, session_name)
            ):
                os.write(master, command)
                command_sent = True
            if any(
                path.is_file() and path.read_bytes().rstrip(b"\r\n") == marker_bytes
                for path in guest_proof_paths(proof_name)
            ):
                observed = True
                break
            if process.poll() is not None:
                break
            if time.monotonic() >= poll_state_after:
                poll_state_after = time.monotonic() + SESSION_STATE_POLL_INTERVAL
                boot_failure = session_state_failure(capsem, session_name, timeout)
                if boot_failure is not None:
                    break

        if not observed:
            # One last look: the loop also ends when the TUI exits or the
            # deadline passes, and either can be the tail end of a boot the
            # service has already given up on.
            if boot_failure is None:
                boot_failure = session_state_failure(capsem, session_name, timeout)
            if boot_failure is not None:
                raise RuntimeError(f"guest shell is unreachable; {boot_failure}")
            tail = bytes(output[-4000:]).decode("utf-8", errors="replace")
            failure = (
                "guest shell marker was not observed before timeout"
                if command_sent
                else "guest shell prompt was not observed before timeout"
            )
            raise RuntimeError(
                f"{failure}; terminal tail follows:\n{tail}"
            )

        # Exit the guest shell, then use the TUI's global Alt-Q shortcut.
        with contextlib.suppress(OSError):
            os.write(master, b"exit\r")
        time.sleep(0.5)
        if process.poll() is None:
            with contextlib.suppress(OSError):
                os.write(master, b"\x1bq")
    finally:
        os.close(master)
        stop_process(process)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capsem", type=Path, required=True)
    parser.add_argument("--marker", required=True)
    parser.add_argument("--session-name", default=f"installed-shell-proof-{os.getpid()}")
    parser.add_argument("--profile")
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--startup-delay", type=float, default=2.0)
    parser.add_argument("--keep-session", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    values = [("marker", args.marker), ("session name", args.session_name)]
    if args.profile is not None:
        values.append(("profile", args.profile))
    for label, value in values:
        if not SAFE_VALUE.fullmatch(value):
            raise SystemExit(f"{label} contains unsupported characters: {value!r}")
    if args.timeout <= 0 or args.startup_delay < 0:
        raise SystemExit("timeout values must be positive")
    if not args.capsem.is_file() or not os.access(args.capsem, os.X_OK):
        raise SystemExit(f"capsem executable not found: {args.capsem}")

    try:
        prove_shell(
            args.capsem,
            args.marker,
            args.session_name,
            args.profile,
            args.timeout,
            args.startup_delay,
        )
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"installed shell proof failed: {error}", file=sys.stderr)
        # Only a passing proof deletes its session. The session dir is the
        # sole copy of process.log and serial.log, and `capsem delete` removes
        # exactly the files that explain the failure -- callers copy them out
        # after this process exits. Nothing leaks: stopping the service kills
        # every VM process and keeps persistent session dirs intact.
        print(
            f"preserving session {args.session_name} for post-mortem "
            "(process.log and serial.log stay in its session dir)",
            file=sys.stderr,
        )
        return 1
    if not args.keep_session:
        with contextlib.suppress(OSError, subprocess.TimeoutExpired):
            subprocess.run(
                [str(args.capsem), "delete", args.session_name],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=min(args.timeout, 30),
            )
    print(f"installed shell proof passed: {args.marker}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
