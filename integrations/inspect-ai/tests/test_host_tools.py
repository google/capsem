"""Host tools must preserve the Linux semantics used by guest-backed tests."""

import subprocess as sp
from pathlib import Path


def test_host_backed_guest_stat_reports_actual_permissions(tmp_path: Path) -> None:
    target = tmp_path / "mode-sensitive file"
    target.write_bytes(b"data")
    target.chmod(0o200)
    result = sp.run(["stat", "-c", "%a:%s", str(target)], capture_output=True, text=True, check=False)
    assert (result.returncode, result.stdout.strip()) == (0, "200:4"), (
        "Host-backed Linux guest tests need real GNU stat (brew install coreutils on macOS); "
        "BSD stat silently triggered the guest permission probe's fallback mode 644."
    )
