"""Own every Debian packaging resource under the Linux packaging rail."""

from __future__ import annotations

import tomllib
from pathlib import Path

from helpers.source_modes import tracked_source_modes

ROOT = Path(__file__).resolve().parents[3]
LINUX = ROOT / "build_system" / "packaging" / "linux"
LEGACY = ROOT / ("scr" + "ipts")

EXPECTED_MODES = {
    "99-capsem-vm-devices.rules": 0o644,
    "build-linux-package.sh": 0o644,
    "deb-postinst.sh": 0o755,
    "deb-preinst.sh": 0o755,
    "derive-deb-libc-floor.py": 0o644,
    "install-deb-runtime-dependencies.py": 0o644,
    "prove-deb-platform-support.py": 0o644,
    "repack-deb.sh": 0o755,
    "select-linux-deb-proof.sh": 0o755,
}


def test_linux_packaging_resources_have_one_exact_owner() -> None:
    found = {
        path.name
        for path in LINUX.iterdir()
        if path.is_file() and not path.name.startswith(".")
    }

    assert found == set(EXPECTED_MODES)
    assert all(not (LEGACY / name).exists() for name in EXPECTED_MODES)


def test_linux_packaging_resources_preserve_reviewed_modes() -> None:
    assert tracked_source_modes(ROOT, LINUX) == EXPECTED_MODES


def test_gate_selects_every_linux_packaging_entrypoint_from_its_owner() -> None:
    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    prefix = "build_system/packaging/linux/"

    assert config["package"]["build_script"] == prefix + "build-linux-package.sh"
    assert config["package"]["proof_selector"] == prefix + "select-linux-deb-proof.sh"
    assert config["install"]["suite"]["runtime_dependencies_script"] == (
        prefix + "install-deb-runtime-dependencies.py"
    )
    assert config["modules"]["platform_support_script"] == (
        prefix + "prove-deb-platform-support.py"
    )

    discarded = {
        row["where"]
        for row in config["boundary"]["discarded_verdicts"]
        if row["where"].endswith(("deb-preinst.sh", "deb-postinst.sh"))
    }
    assert discarded == {
        prefix + "deb-preinst.sh",
        prefix + "deb-postinst.sh",
    }


def test_linux_package_assembly_uses_package_relative_resources() -> None:
    build = (LINUX / "build-linux-package.sh").read_text(encoding="utf-8")
    repack = (LINUX / "repack-deb.sh").read_text(encoding="utf-8")
    preinstall = (LINUX / "deb-preinst.sh").read_text(encoding="utf-8")
    postinstall = (LINUX / "deb-postinst.sh").read_text(encoding="utf-8")

    assert 'bash "$SCRIPT_DIR/repack-deb.sh"' in build
    assert '"$SCRIPT_DIR/derive-deb-libc-floor.py"' in repack
    assert '"$SCRIPT_DIR/deb-preinst.sh"' in repack
    assert '"$SCRIPT_DIR/deb-postinst.sh"' in repack
    assert '"$SCRIPT_DIR/../shared/$helper"' in repack
    for maintainer_script in (preinstall, postinstall):
        assert '"$(dirname "$0")/../shared/' in maintainer_script


def test_linux_package_assembly_uses_cargos_configured_target_root() -> None:
    build = (LINUX / "build-linux-package.sh").read_text(encoding="utf-8")

    assert ': "${CARGO_HOME:?}" "${CARGO_TARGET_DIR:?}"' in build
    assert 'RELEASE_DIR="$CARGO_TARGET_DIR/$RUST_TARGET/release"' in build
    assert 'AGENT_DIR="$CARGO_TARGET_DIR/build/linux-agent/$TARGET_ARCH"' in build
    assert "/cargo-cache/target" not in build


def test_release_workflow_selects_owned_runtime_dependency_helper() -> None:
    source = (ROOT / ".github" / "workflows" / "release.yaml").read_text(
        encoding="utf-8"
    )
    owned = "build_system/packaging/linux/install-deb-runtime-dependencies.py"
    legacy = "scr" + "ipts/install-deb-runtime-dependencies.py"

    assert source.count(owned) == 2
    assert legacy not in source


def test_debian_package_owns_immediate_vm_device_access() -> None:
    repack = (LINUX / "repack-deb.sh").read_text(encoding="utf-8")
    postinstall = (LINUX / "deb-postinst.sh").read_text(encoding="utf-8")
    helper = (
        ROOT / "build_system/packaging/shared/install-vm-device-access"
    ).read_text(encoding="utf-8")
    rules = (LINUX / "99-capsem-vm-devices.rules").read_text(encoding="utf-8")

    assert 'embed_pkg_script install-vm-device-access "$WORK_DIR/deb/DEBIAN/postinst"' in repack
    assert 'for dependency in libxdo3 acl kmod udev; do' in repack
    assert 'capsem_install_vm_device_access "$TARGET_USER"' in postinstall
    assert 'MODE="0666"' not in helper
    assert 'target_uid=$(id -u "$target_user")' in helper
    assert 'RUN:="/usr/bin/setfacl -m u:%s:rw /dev/%%k"' in helper
    assert 'rule_target=/run/udev/rules.d/99-capsem-vm-devices.rules' in helper
    assert 'rm -f /etc/udev/rules.d/99-capsem-vm-devices.rules' in helper
    assert 'install -Dm0644 "$rule_temp" "$rule_target"' in helper
    assert helper.index("udevadm settle --timeout=10") < helper.index(
        'setfacl -m "u:$target_user:rw"'
    )
    assert 'setfacl -m "u:$target_user:rw" "$device"' in helper
    assert "runuser -u \"$target_user\" -- sh -c" in helper
    assert '\'test -r "$1" && test -w "$1"\'' in helper
    assert 'KERNEL=="kvm", GROUP="kvm", MODE="0660", TAG-="uaccess"' in rules
    assert 'KERNEL=="vhost-vsock", GROUP="kvm", MODE="0660", TAG-="uaccess"' in rules
