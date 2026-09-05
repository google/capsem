"""Release workflows must not hide installer permissions behind open devices."""

from pathlib import Path

import yaml
from capsem_builder.gate.shellnodes import commands
from capsem_builder.gate.shellparse import parse
from capsem_builder.gate.shellsurfaces import workflow_bodies

ROOT = Path(__file__).resolve().parents[2]
VM_DEVICES = frozenset({"/dev/kvm", "/dev/vhost-vsock"})
WORLD_WRITABLE_RATIONALE = """\
The installed-package glowup must prove that postinstall grants the service
access to VM devices. A workflow-level chmod 0666 makes the package look usable
even when its systemd user manager has stale supplementary groups.
"""


def test_install_runner_provisions_access_before_install_preflight() -> None:
    workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yaml").read_text())
    steps = workflow["jobs"]["test-install"]["steps"]
    provisioning = [
        index
        for index, step in enumerate(steps)
        if any(
            "build_system/packaging/shared/install-vm-device-access" in command.argv
            for command in commands(parse(step.get("run", "")))
        )
    ]
    install = next(
        index
        for index, step in enumerate(steps)
        if step.get("name") == "Run install e2e tests"
    )
    assert provisioning and max(provisioning) < install, (
        "Checking device existence does not grant the runner read/write access; "
        "use the shared restricted-access provisioner before install preflight.\n"
        + WORLD_WRITABLE_RATIONALE
    )


def test_release_workflows_do_not_make_vm_devices_world_writable() -> None:
    violations: list[str] = []
    for where, body in workflow_bodies(ROOT / ".github" / "workflows").items():
        for command in commands(parse(body, origin=where)):
            chmod_open = (
                command.program == "chmod"
                and "0666" in command.argv
                and VM_DEVICES.intersection(command.argv)
            )
            open_device_rule = any(
                'MODE="0666"' in argument
                and ('KERNEL=="kvm"' in argument or 'KERNEL=="vhost-vsock"' in argument)
                for argument in command.argv
            )
            if chmod_open or open_device_rule:
                violations.append(f"{where}: {' '.join(command.argv)}")

    bootstrap = (
        ROOT / "build_system/scripts/bootstrap/bootstrap-linux-host.sh"
    ).read_text(encoding="utf-8")
    helper = (
        ROOT / "build_system/packaging/shared/install-vm-device-access"
    ).read_text(encoding="utf-8")
    assert "chmod 0666 /dev/kvm" not in bootstrap, WORLD_WRITABLE_RATIONALE
    assert 'MODE="0666"' not in bootstrap, WORLD_WRITABLE_RATIONALE
    assert 'MODE="0666"' not in helper, WORLD_WRITABLE_RATIONALE
    assert not violations, f"{violations}\n{WORLD_WRITABLE_RATIONALE}"


def test_vm_device_acl_is_reapplied_after_every_udev_event() -> None:
    helper = (
        ROOT / "build_system/packaging/shared/install-vm-device-access"
    ).read_text(encoding="utf-8")

    assert 'target_uid=$(id -u "$target_user")' in helper
    assert helper.count('RUN:="/usr/bin/setfacl -m u:%s:rw /dev/%%k"') == 2, (
        "KVM emits change events on every VM create/destroy. Udev retains "
        "historical uaccess tags, so TAG-= cannot cancel the queued seat ACL "
        "reset. Replace and finalize that queue with the owned ACL grant; "
        "RUN+= leaves an intermittent EACCES window for stale user managers."
    )
    assert "udevadm settle --timeout=10" in helper
