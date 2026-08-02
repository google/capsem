"""What a package build is told, computed rather than performed.

Pure functions: the pinned toolchain, the channel a build is for, and the
environment the builder container receives. Separate from the rail that runs
the build because they can be asserted directly -- a rename in
`config/gate.toml` fails a unit test here instead of producing a package built
against the wrong manifest, which is a thing you find out much later.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

from . import config as gate_config
from . import host
from .errors import GateError


def pinned_toolchain(root: Path) -> str:
    """The Rust version `rust-toolchain.toml` pins, read rather than repeated.

    It was spelled three times inside one inline shell script, which is three
    chances for a toolchain bump to leave the package rail behind.
    """
    pin = Path(root) / gate_config.for_root(root).package.toolchain_pin
    try:
        return tomllib.loads(pin.read_text(encoding="utf-8"))["toolchain"]["channel"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        raise GateError(f"{pin} declares no [toolchain] channel: {exc}") from None


def resolve_channel(channel: str, config: gate_config.GateConfig) -> str:
    allowed = config.package.channels
    if channel not in allowed:
        raise GateError(
            f"CAPSEM_INSTALL_CHANNEL must be one of {', '.join(allowed)} (got: {channel})"
        )
    return channel


def package_environment(
    config: gate_config.GateConfig,
    target,
    *,
    toolchain: str,
    manifest_url: str,
    signing: dict[str, str],
) -> dict[str, str]:
    """What the builder container is told, as a value rather than a side effect.

    Pure, so the names can be asserted directly instead of by reading a
    `docker run` argv back out of a recording runner -- and so a rename in
    `config/gate.toml` is a test that fails here rather than a package that
    builds against the wrong manifest.
    """
    uid, gid = host.user()
    return {
        "TARGET_ARCH": target.name,
        "RUST_TARGET": target.rust_target,
        "DPKG_ARCH": target.dpkg,
        "RUST_TOOLCHAIN": toolchain,
        "PKG_CONFIG_PATH": target.pkg_config_path,
        config.package.manifest_variable: manifest_url,
        "HOST_UID": str(uid),
        "HOST_GID": str(gid),
        **signing,
    }
