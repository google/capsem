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


def test_graphical_app_starts_after_client_display_negotiation(tmp_path):
    binary = tmp_path / "bin"
    binary.mkdir()
    for command in ("gnome-keyring-daemon", "certutil"):
        stub = binary / command
        stub.write_text("#!/bin/sh\nexit 0\n")
        stub.chmod(0o755)
    digest = binary / "sha256sum"
    digest.write_text("#!/bin/sh\ncat >/dev/null\nprintf '%064d  -\\n' 0\n")
    digest.chmod(0o755)
    arguments = tmp_path / "arguments"
    xpra = binary / "xpra"
    xpra.write_text(
        f'#!/bin/sh\nprintf "%s\\n" "$@" > "{arguments}"\n'
        'echo 0 > "$XDG_RUNTIME_DIR/application.status"\n'
    )
    xpra.chmod(0o755)
    bundle = tmp_path / "empty-test-ca"
    bundle.write_text("")
    result = subprocess.run(
        ["sh", str(ROOT / "images/base/gui-entrypoint.sh")],
        env={**os.environ, "PATH": f"{binary}:/usr/bin:/bin", "HOME": str(tmp_path / "home"),
             "XDG_RUNTIME_DIR": str(tmp_path / "runtime"), "SSL_CERT_FILE": str(bundle),
             "CAPSEM_GUI_BUS": "1", "DBUS_SESSION_BUS_ADDRESS": "test-bus",
             "CAPSEM_GUI_APP": "/test-app"},
        capture_output=True, timeout=10, check=False,
    )
    assert result.returncode == 0, result.stderr
    actual = arguments.read_text().splitlines()
    assert f"--start-child-after-connect={ROOT}/images/base/gui-entrypoint.sh --child" in actual
    assert not any(argument.startswith("--start-child=") for argument in actual)
    assert "--exit-with-children=yes" in actual
