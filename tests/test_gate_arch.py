"""One architecture record, replacing four disagreeing shell `case` blocks."""

from __future__ import annotations

import pytest

from capsem.gate import arch
from capsem.gate.errors import GateError


@pytest.mark.parametrize(
    "spelling, expected",
    [
        ("arm64", arch.ARM64),
        ("aarch64", arch.ARM64),
        ("AArch64", arch.ARM64),
        (" arm64 ", arch.ARM64),
        ("x86_64", arch.X86_64),
        ("amd64", arch.X86_64),
    ],
)
def test_every_accepted_spelling_reaches_one_record(spelling: str, expected: arch.Arch) -> None:
    assert arch.resolve(spelling) is expected


def test_intel_is_x86_64_to_capsem_and_amd64_to_dpkg() -> None:
    """The one mapping the shell got right in one recipe and had to repeat.

    `_gate-install` set `TARGET_ARCH=x86_64` and `DEB_ARCH=amd64` from the same
    `case` arm. Nothing forced the next copy of that `case` to keep the two
    names distinct, and a copy that used `x86_64` for both would have looked
    for a package Debian never names that way.
    """
    assert arch.X86_64.name == "x86_64"
    assert arch.X86_64.dpkg == "amd64"
    assert arch.ARM64.name == arch.ARM64.dpkg == "arm64"


def test_rust_triples_are_the_linux_gnu_targets_the_builder_installs() -> None:
    assert arch.ARM64.rust_target == "aarch64-unknown-linux-gnu"
    assert arch.X86_64.rust_target == "x86_64-unknown-linux-gnu"


def test_pkg_config_path_is_derived_not_restated() -> None:
    assert arch.ARM64.pkg_config_path == (
        "/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
    )
    assert arch.X86_64.pkg_config_path.startswith("/usr/lib/x86_64-linux-gnu/")


def test_an_unsupported_architecture_names_itself_and_the_alternatives() -> None:
    with pytest.raises(GateError) as failure:
        arch.resolve("riscv64")

    message = str(failure.value)
    assert "riscv64" in message
    assert "arm64" in message and "x86_64" in message


def test_host_resolves_on_this_machine() -> None:
    assert arch.host() in arch.SUPPORTED
