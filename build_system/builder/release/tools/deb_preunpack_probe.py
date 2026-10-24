"""The existing distro matrix's original-package, pre-unpack admission check.

dpkg owns dependency resolution and ordering. This does not parse Debian
relations, configure the package, start a service, or install tools. A fresh
disposable account exercises the candidate's real private admission path;
existing/incompatible ledgers remain the installed-package proof's concern.
"""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

from . import preinstall_resource as resource
from .generate_host_binary_sbom import deb_data_member
from .package_payload import deb_field, tar_payload_files


def canonical_preinst(repository: Path) -> bytes:
    """Expected original body, in the existing repack-deb embedding order."""
    shared = repository / "build_system/packaging/shared"
    body = (repository / "build_system/packaging/linux/deb-preinst.sh").read_bytes()
    for helper in ("install-diagnostics", "service-owned-update", "retire-cohort"):
        body = resource.SHEBANG + (shared / helper).read_bytes().split(b"\n", 1)[1] + body[len(resource.SHEBANG):]
    return body


def validate_package(package: Path, *, repository: Path) -> str:
    """Bind the original embedded resource to this package's actual ELF bytes.

    Reuse portable package readers and the canonical resource verifier. Only
    allowlisted regular members are materialized, never archive paths.
    """
    limits = resource.load_limits(repository)
    control_name, control_data = deb_data_member(package, member_prefix="control.tar")
    controls = tar_payload_files(control_data, control_name, select=lambda path: path in {"/control", "/preinst"})
    data_name, data = deb_data_member(package)
    binaries = tar_payload_files(data, data_name, select=lambda path: path == "/usr/bin/capsem-ledger")
    selected = {
        "DEBIAN/control": (controls.get("/control"), limits.max_control_bytes),
        "DEBIAN/preinst": (controls.get("/preinst"), limits.max_script_bytes),
        "usr/bin/capsem-ledger": (binaries.get("/usr/bin/capsem-ledger"), limits.max_binary_bytes),
    }
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        for name, (contents, maximum) in selected.items():
            if contents is None or not 0 < len(contents) <= maximum:
                raise ValueError(f"platform proof requires bounded package member: {name}")
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
            path.chmod(0o555 if name.startswith("usr/") else 0o644)
        deb_field(package, "Pre-Depends")
        if deb_field(package, "Package") != "capsem":
            raise ValueError("platform proof requires the canonical capsem package identity")
        resource.verify(root, repository=repository, limits=limits)
        original = canonical_preinst(repository)[len(resource.SHEBANG):]
        if (root / "DEBIAN/preinst").read_bytes().partition(resource.END)[2] != original:
            raise ValueError("platform proof requires the exact original canonical preinst body")
        return resource.candidate(root, limits).architecture


def receipt(supported: bool) -> bytes:
    return b"CAPSEM_PLATFORM_PREUNPACK_" + (b"OK\n" if supported else b"REFUSED\n")


_SCRIPT = r"""set -eu
export LC_ALL=C
for tool in dpkg dpkg-deb dpkg-query useradd runuser bash base64 timeout sha256sum pgrep ps getent su tee find grep id cut; do
    command -v "$tool" >/dev/null || { echo "missing platform prerequisite: $tool" >&2; exit 1; }
done
mkdir /probe
cat > /probe/candidate.deb
dpkg --version
dpkg-deb --version
dpkg-deb --info /probe/candidate.deb
useradd --create-home --shell /bin/bash capsem-platform-probe
test "$(id -u capsem-platform-probe)" -gt 0
probe_home=$(getent passwd capsem-platform-probe | cut -d: -f6)
case "$probe_home" in /*) ;; *) echo 'probe account has no absolute home' >&2; exit 1 ;; esac
test ! -e /usr/bin/capsem-ledger
test ! -e "$probe_home/.capsem"
status=0
SUDO_USER=capsem-platform-probe dpkg --unpack /probe/candidate.deb > /probe/unpack.log 2>&1 || status=$?
cat /probe/unpack.log
if [ "$1" = supported ]; then
    test "$status" -eq 0
    test -x /usr/bin/capsem-ledger
    test "$(dpkg-query --show --showformat='${db:Status-Status}' capsem)" = unpacked
    test -f "$probe_home/.capsem/logs/install.log"
    test ! -e "$probe_home/.capsem/sessions/host.db"
    test -z "$(find /var/tmp -maxdepth 1 -name 'capsem-preinstall.*' -print -quit)"
    printf 'CAPSEM_PLATFORM_PREUNPACK_OK\n'
else
    test "$status" -ne 0
    grep -F 'pre-dependency problem' /probe/unpack.log
    test ! -e /usr/bin/capsem-ledger
    test ! -e "$probe_home/.capsem"
    test -z "$(find /var/tmp -maxdepth 1 -name 'capsem-preinstall.*' -print -quit)"
    printf 'CAPSEM_PLATFORM_PREUNPACK_REFUSED\n'
fi
"""


def check_preunpack(image: str, package: Path, architecture: str, supported: bool) -> list[str]:
    """One existing matrix image; no host mounts, network or package configure."""
    with package.open("rb") as original:
        result = subprocess.run(
            (
                "docker", "run", "--rm", "-i", "--pull=never", "--network=none",
                "--platform", f"linux/{architecture}", "--entrypoint", "sh", image,
                "-c", _SCRIPT, "platform-preunpack", "supported" if supported else "refused",
            ),
            stdin=original, capture_output=True,
        )
    if result.returncode != 0 or not result.stdout.endswith(receipt(supported)):
        output = (result.stdout + result.stderr).decode(errors="replace").strip()
        return [f"{image}: original package pre-unpack proof failed (exit {result.returncode}):\n{output}"]
    print(f"  ok  {image} original candidate admission/pre-dependency ordering")
    return []
