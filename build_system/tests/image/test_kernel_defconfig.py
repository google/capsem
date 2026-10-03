"""Contract over the checked-in guest kernel defconfigs.

The kernel build fails when the kernel does not honor a pin; these tests
fail when a pin a workload depends on is removed. Options are pinned per
architecture so a regression on one arch cannot hide behind the other.
"""

from __future__ import annotations

from pathlib import Path

import pytest

KERNEL_DIR = Path(__file__).resolve().parents[3] / "config" / "docker" / "image" / "kernel"
ARCHES = ("arm64", "x86_64")

# The OCI workload runs in these namespaces; USER_NS lets it hold its
# capabilities over a mapped uid range instead of over the VM (#289).
REQUIRED = ("NAMESPACES", "USER_NS", "UTS_NS", "IPC_NS", "PID_NS", "NET_NS", "SECCOMP_FILTER")

# User namespaces expose more of the kernel to the workload, which makes
# keeping the historically richest escalation surfaces compiled out matter more.
FORBIDDEN = ("IO_URING", "BPF_SYSCALL", "USERFAULTFD", "MODULES")


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
