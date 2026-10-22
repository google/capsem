"""The shared graphical child keeps the selected app's arguments and exit status."""

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("exit_status", [0, 73])
def test_graphical_child_reports_the_application_result(tmp_path, exit_status):
    binary = tmp_path / "bin"
    binary.mkdir()
    dbus = binary / "dbus-update-activation-environment"
    dbus.write_text("#!/bin/sh\nexit 0\n")
    dbus.chmod(0o755)
    app = tmp_path / "selected IDE"
    arguments = tmp_path / "arguments"
    app.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > "{arguments}"\nexit {exit_status}\n')
    app.chmod(0o755)
    runtime = tmp_path / "runtime"
    result = subprocess.run(
        ["sh", str(ROOT / "images/base/gui-entrypoint.sh"), "--child"],
        env={**os.environ, "PATH": f"{binary}:/usr/bin:/bin", "XDG_RUNTIME_DIR": str(runtime),
             "CAPSEM_GUI_APP": str(app)},
        capture_output=True,
        timeout=10,
        check=False,
    )
    assert result.returncode == exit_status, result.stderr
    assert (runtime / "application.status").read_text().strip() == str(exit_status)
    assert arguments.read_text().splitlines() == ["--password-store=gnome-libsecret"]
