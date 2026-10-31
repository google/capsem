"""Validate the stapled Apple ticket; never turn a failed check into success."""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path

from ...gate.config import load
from ...gate.releaseschema import AppleTicketValidationConfig


def validate(package: Path, settings: AppleTicketValidationConfig) -> int:
    for attempt in range(1, settings.attempts + 1):
        try:
            result = subprocess.run(
                ["xcrun", "stapler", "validate", str(package)],
                stdin=subprocess.DEVNULL,
                capture_output=True,
                text=True,
                timeout=settings.timeout_seconds,
                check=False,
            )
        except subprocess.TimeoutExpired:
            print("Apple ticket validation timed out", file=sys.stderr, flush=True)
            return 124
        print(result.stdout, end="", flush=True)
        print(result.stderr, end="", file=sys.stderr, flush=True)
        output = result.stdout + result.stderr
        lost_cloudkit_connection = (
            result.returncode == 68
            and "Error Domain=NSURLErrorDomain Code=-1005" in output
            and "https://api.apple-cloudkit.com/" in output
        )
        if not lost_cloudkit_connection or attempt == settings.attempts:
            return result.returncode
        delay = settings.retry_delay_seconds * attempt
        print(
            f"Apple CloudKit connection lost; retrying required ticket validation "
            f"({attempt + 1}/{settings.attempts}) after {delay:g}s",
            file=sys.stderr,
            flush=True,
        )
        time.sleep(delay)
    raise AssertionError("configuration requires at least one validation attempt")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()
    return validate(args.package, load(Path.cwd()).release.apple_ticket_validation)


if __name__ == "__main__":
    raise SystemExit(main())
