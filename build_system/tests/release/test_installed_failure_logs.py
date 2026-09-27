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


def test_help_needs_no_home_directory(monkeypatch, capsys) -> None:
    """The verifier runs staged in a bare guest and under service managers.

    An OS Login account has no /etc/passwd entry, so with HOME unset
    Path.home() raises; the parser used it for a default and failed on
    `--help` before any option was even read.
    """
    import pytest
    from capsem_builder.release.tools import verify_installed_release

    def unresolvable() -> Path:
        raise RuntimeError("Could not determine home directory.")

    monkeypatch.setattr(Path, "home", staticmethod(unresolvable))
    monkeypatch.setattr("sys.argv", ["verify-installed-release.py", "--help"])
    with pytest.raises(SystemExit) as exited:
        verify_installed_release.main()
    assert exited.value.code == 0
    assert "--capsem-home" in capsys.readouterr().out
