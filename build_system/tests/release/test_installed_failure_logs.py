"""Installed release proof for post-mortem session logs."""

from pathlib import Path

from capsem_builder.release.tools.verify_installed_release import (
    verify_failed_session_logs,
)


def test_installed_probe_reads_and_removes_preserved_failure(tmp_path: Path) -> None:
    capsem_home = tmp_path / "home"
    capsem = tmp_path / "capsem"
    capsem.write_text(
        """#!/usr/bin/env python3
import os
from pathlib import Path
import sys

session_id = sys.argv[2]
root = Path(os.environ["CAPSEM_RUN_DIR"]) / "sessions"
failed = next(root.glob(f"{session_id}-failed-*"))
print((failed / "process.log").read_text(), end="")
""",
        encoding="utf-8",
    )
    capsem.chmod(0o755)

    verify_failed_session_logs(capsem, capsem_home)

    sessions = capsem_home / "run" / "sessions"
    assert list(sessions.iterdir()) == []
