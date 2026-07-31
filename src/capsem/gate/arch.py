"""One target architecture, under every name the toolchain gives it.

`uname -m` says `aarch64`; the assets tree says `arm64`; dpkg says `arm64` for
ARM but `amd64` for Intel; rustc says `aarch64-unknown-linux-gnu`. Four shell
recipes each carried a `case` over some subset of those spellings, so each was
free to disagree with the rest -- and they did: `_cross-compile` accepted only
`arm64`/`x86_64` while `_gate-install` also had to derive the dpkg name, and
nothing tied the two together.

Naming all four spellings on one record makes the mapping a lookup instead of a
convention, and makes an unsupported host a single error message.

`host_system` lives here for the same reason: the gate branches on Darwin
versus Linux in five places, and each `uname -s` was its own chance to spell
the answer differently.
"""

from __future__ import annotations

import platform
from dataclasses import dataclass

from .errors import GateError


@dataclass(frozen=True)
class Arch:
    """A build target under each name some tool insists on."""

    name: str
    """Capsem's own spelling: `assets/<name>`, `capsem-host-target-<name>`."""

    rust_target: str
    """The triple for `cargo build --target` and `rustup target add`."""

    dpkg: str
    """The Debian `Architecture:` field, and the `.deb` filename suffix."""

    gnu: str
    """The multiarch tuple naming this target's library directory."""

    @property
    def pkg_config_path(self) -> str:
        """Where the cross toolchain's `.pc` files live inside the builder."""
        return f"/usr/lib/{self.gnu}/pkgconfig:/usr/share/pkgconfig"


ARM64 = Arch(
    name="arm64",
    rust_target="aarch64-unknown-linux-gnu",
    dpkg="arm64",
    gnu="aarch64-linux-gnu",
)
X86_64 = Arch(
    name="x86_64",
    rust_target="x86_64-unknown-linux-gnu",
    dpkg="amd64",
    gnu="x86_64-linux-gnu",
)

SUPPORTED: tuple[Arch, ...] = (ARM64, X86_64)

# Every spelling a host kernel or an operator may hand us, folded onto the one
# record. `uname -m` supplies the left column on some machine we support.
_ALIASES: dict[str, Arch] = {
    "arm64": ARM64,
    "aarch64": ARM64,
    "x86_64": X86_64,
    "amd64": X86_64,
}


def resolve(spelling: str) -> Arch:
    """The architecture named by any accepted spelling of it."""
    try:
        return _ALIASES[spelling.strip().lower()]
    except KeyError:
        raise GateError(
            f"unsupported architecture {spelling!r}; "
            f"expected one of {', '.join(sorted(_ALIASES))}"
        ) from None


def host() -> Arch:
    """The architecture of the machine running the gate."""
    return resolve(platform.machine())


def host_system() -> str:
    """The operating system the gate is running on: `Darwin` or `Linux`."""
    return platform.system()


def on_macos() -> bool:
    return host_system() == "Darwin"


def on_linux() -> bool:
    return host_system() == "Linux"
