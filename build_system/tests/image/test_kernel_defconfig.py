"""Contract over the checked-in guest kernel defconfigs.

The kernel build fails when the kernel does not honor a pin; these tests
fail when a pin a workload depends on is removed. Options are pinned per
architecture so a regression on one arch cannot hide behind the other.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest

KERNEL_DIR = Path(__file__).resolve().parents[3] / "config" / "docker" / "image" / "kernel"
ARCHES = ("arm64", "x86_64")

# The OCI workload runs in these namespaces; USER_NS lets it hold its
# capabilities over a mapped uid range instead of over the VM (#289).
REQUIRED = ("NAMESPACES", "USER_NS", "UTS_NS", "IPC_NS", "PID_NS", "NET_NS", "SECCOMP_FILTER")

# User namespaces expose more of the kernel to the workload, which makes
# keeping the historically richest escalation surfaces compiled out matter more.
FORBIDDEN = (
    "IO_URING",
    "BPF_SYSCALL",
    "USERFAULTFD",
    "MODULES",
    # Surface the kernel defaults used to switch on without anyone deciding.
    "LEGACY_TIOCSTI",
    "CROSS_MEMORY_ATTACH",
    "COREDUMP",
    "FTRACE",
    "BLK_DEV_WRITE_MOUNTED",
)

# Legacy x86 entry points: vsyscall page, per-process LDT, userspace port
# I/O, 16-bit segments.
X86_FORBIDDEN = (
    "X86_VSYSCALL_EMULATION",
    "MODIFY_LDT_SYSCALL",
    "X86_IOPL_IOPERM",
    "X86_16BIT",
)
TEMPLATE = KERNEL_DIR.parents[1] / "Dockerfile.kernel.j2"


def defconfig(arch: str) -> dict[str, str]:
    options = {}
    for line in (KERNEL_DIR / f"defconfig.{arch}").read_text().splitlines():
        if line.startswith("CONFIG_") and "=" in line:
            key, value = line.removeprefix("CONFIG_").split("=", 1)
            options[key] = value
    return options


# Google Antigravity CLI's ARM64 binary uses TCMalloc and assumes a 48-bit
# userspace VA layout. The pin was dropped once in a config rewrite and the
# 6.18 default is 52-bit with 5-level tables.
@pytest.mark.parametrize(
    ("option", "value"),
    [("ARM64_4K_PAGES", "y"), ("ARM64_VA_BITS_48", "y"), ("ARM64_VA_BITS", "48")],
)
def test_arm64_keeps_the_userspace_layout_agy_needs(option: str, value: str) -> None:
    assert defconfig("arm64").get(option) == value, (
        f"defconfig.arm64 must set CONFIG_{option}={value}"
    )


@pytest.mark.parametrize("arch", ARCHES)
@pytest.mark.parametrize("option", REQUIRED)
def test_required_option_is_built_in(arch: str, option: str) -> None:
    assert defconfig(arch).get(option) == "y", f"defconfig.{arch} must set CONFIG_{option}=y"


@pytest.mark.parametrize("arch", ARCHES)
@pytest.mark.parametrize("option", FORBIDDEN)
def test_forbidden_option_is_compiled_out(arch: str, option: str) -> None:
    assert defconfig(arch).get(option, "n") == "n", (
        f"defconfig.{arch} must not enable CONFIG_{option}"
    )


@pytest.mark.parametrize("option", X86_FORBIDDEN)
def test_x86_legacy_entry_point_is_compiled_out(option: str) -> None:
    assert defconfig("x86_64").get(option) == "n", f"defconfig.x86_64 must pin CONFIG_{option}=n"


def test_the_kernel_is_built_from_allnoconfig() -> None:
    """olddefconfig filled ~300 unpinned options per arch with upstream defaults."""
    text = TEMPLATE.read_text()
    assert "KCONFIG_ALLCONFIG=/tmp/capsem.defconfig allnoconfig" in text
    assert "make olddefconfig" not in text


def test_workloads_can_read_their_own_memory_map() -> None:
    """Redis exits on arm64 when /proc/self/smaps is missing."""
    for arch in ("arm64", "x86_64"):
        assert defconfig(arch).get("PROC_PAGE_MONITOR") == "y", arch


# The kernel Dockerfile copies kernel/patches/ whole, so a patch on disk that
# build.toml does not list would sit in the build context unapplied, looking
# like part of the kernel. Listed and present must be the same set.
def test_every_kernel_patch_on_disk_is_listed():
    listed = tomllib.loads((KERNEL_DIR.parent / "build.toml").read_text())["build"]["kernel"][
        "patches"
    ]
    on_disk = sorted(f"kernel/patches/{p.name}" for p in (KERNEL_DIR / "patches").iterdir())
    assert sorted(listed) == on_disk


# Apple's virtio-fs server stores POSIX ACL xattrs without applying them, so
# cp -a into /root on macOS dropped group and other bits (0640 became 0600).
# The guest kernel refuses ACLs for FUSE daemons that did not negotiate
# FUSE_POSIX_ACL; tools fall back to chmod. See the patch header.
def test_fuse_acl_patch_refuses_acls_without_fuse_posix_acl():
    patch = (
        KERNEL_DIR / "patches" / "0001-fuse-refuse-posix-acls-without-fuse-posix-acl.patch"
    ).read_text()
    assert "+++ b/fs/fuse/acl.c" in patch
    assert "-\treturn !fc->posix_acl && (i_user_ns(inode) != &init_user_ns);" in patch
    assert "+\treturn !fc->posix_acl;" in patch


# Apple's virtio-fs server never negotiates FUSE_ALLOW_IDMAP, so idmapping the
# workspace failed with EINVAL and a user-namespaced workload saw it owned by
# nobody. virtio-fs always has default_permissions; the patch clears
# SB_I_NOIDMAP only under it, and keeps request headers on the caller's ids
# for daemons that did not opt in. See the patch header.
def test_virtiofs_idmap_patch_keeps_classic_headers_for_unaware_daemons():
    patch = (KERNEL_DIR / "patches" / "0002-virtiofs-allow-idmapped-mounts.patch").read_text()
    assert "+++ b/fs/fuse/virtio_fs.c" in patch
    assert "+\tif (fc->default_permissions) {\n+\t\tsb->s_iflags &= ~SB_I_NOIDMAP;" in patch
    # The daemon keeps the caller's own ids unless it negotiated idmap
    # support: Apple's refuses the invalid uid an idmapped request carries.
    assert "+++ b/fs/fuse/dev.c" in patch
    assert "+\t\t\t!fc->idmap_headers;" in patch
    assert "+\t\t\t\t\tfc->idmap_headers = 1;" in patch
    # Apple's daemon reflects the header ids back as the owner; they are the
    # caller's own, so a workload owns what it touches through the idmap.
    assert "+\t\t\t\t\tcurrent_user_ns() : fc->user_ns;" in patch
    assert "+\t\tfc->caller_ns_ids = 1;" in patch
