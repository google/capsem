"""Hermetic in-VM package probes for black-box fork/package tests."""

from __future__ import annotations

from helpers.service import exec_output_text

FORK_PROBE_COMMAND = "capsem-fork-probe"
FORK_PROBE_OUTPUT = "fork-package-ok"
FORK_PROBE_INSTALL_SCRIPT = rf"""set -euo pipefail
rm -rf /tmp/capsem-fork-probe /tmp/capsem-fork-probe.deb
mkdir -p /tmp/capsem-fork-probe/DEBIAN /tmp/capsem-fork-probe/usr/local/bin
cat > /tmp/capsem-fork-probe/DEBIAN/control <<'EOF'
Package: capsem-fork-probe
Version: 1.0
Section: utils
Priority: optional
Architecture: all
Maintainer: Capsem Tests <tests@capsem.local>
Description: Hermetic fork benchmark probe
EOF
cat > /tmp/capsem-fork-probe/usr/local/bin/{FORK_PROBE_COMMAND} <<'EOF'
#!/bin/sh
printf '{FORK_PROBE_OUTPUT}\n'
EOF
chmod 0755 /tmp/capsem-fork-probe/usr/local/bin/{FORK_PROBE_COMMAND}
dpkg-deb --build /tmp/capsem-fork-probe /tmp/capsem-fork-probe.deb >/tmp/capsem-fork-probe.build.log
dpkg -i /tmp/capsem-fork-probe.deb >/tmp/capsem-fork-probe.install.log
{FORK_PROBE_COMMAND}
"""


def install_fork_probe_with_service_client(client, vm_name: str) -> None:
    """Install the fork probe through public service file+exec routes."""
    script_path = "/root/install-capsem-fork-probe.sh"
    write = client.upload_file(vm_name, script_path, FORK_PROBE_INSTALL_SCRIPT, timeout=15)
    assert write and write.get("success") is True, f"probe install script write failed: {write}"

    resp = client.post(
        f"/vms/{vm_name}/exec",
        {"command": f"bash {script_path}", "timeout_secs": 30},
        timeout=40,
    )
    assert resp and resp.get("exit_code") == 0, f"local package install failed: {resp}"
    assert exec_output_text(resp).strip().endswith(FORK_PROBE_OUTPUT), resp

