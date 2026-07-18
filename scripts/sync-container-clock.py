#!/usr/bin/env python3
"""Synchronize Colima's VM clock with a bounded host-side command."""

from __future__ import annotations

import datetime
import subprocess
import sys


def sync_container_clock(*, timeout_seconds: float = 10) -> None:
    timestamp = datetime.datetime.now(datetime.timezone.utc).strftime(
        "%Y-%m-%d %H:%M:%S"
    )
    command = ["colima", "ssh", "--", "sudo", "date", "-u", "-s", timestamp]
    try:
        subprocess.run(
            command,
            check=True,
            text=True,
            capture_output=True,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(
            f"Colima clock synchronization timed out after {timeout_seconds:g} seconds"
        ) from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or str(error)).strip()
        raise RuntimeError(f"Colima clock synchronization failed: {detail}") from error


def main() -> int:
    try:
        sync_container_clock()
    except (OSError, RuntimeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
