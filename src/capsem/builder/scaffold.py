"""Scaffolding for guest image configurations.

Creates directory structures and template TOML files for new guest images,
AI providers, package sets, and MCP servers.
"""

from __future__ import annotations

from pathlib import Path


# ---------------------------------------------------------------------------
# Template content
# ---------------------------------------------------------------------------

_BUILD_TOML = """\
[build]
compression = "zstd"
compression_level = 15

[build.architectures.arm64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/arm64"
rust_target = "aarch64-unknown-linux-musl"
kernel_branch = "6.6"
kernel_image = "arch/arm64/boot/Image"
defconfig = "kernel/defconfig.arm64"
node_major = 24
"""

_DEFCONFIG_STUB = "# Kernel defconfig -- replace with real defconfig\n"

_AI_PROVIDER_TOML = """\
[{name}]
name = "{display_name}"
description = "{display_name} AI provider"
enabled = true

[{name}.api_key]
name = "{display_name} API Key"
env_vars = ["{env_var}"]

[{name}.network]
domains = ["api.{name}.com"]
allow_get = true
allow_post = true
"""

_INSTALL_CMDS = {
    "apt": "apt-get install -y --no-install-recommends",
    "uv": "uv pip install --system --break-system-packages",
    "pip": "pip3 install --break-system-packages",
    "npm": "npm install -g",
}

_PACKAGE_SET_TOML = """\
[{name}]
name = "{display_name}"
manager = "{manager}"
install_cmd = "{install_cmd}"
packages = ["example-package"]
"""

_MCP_STDIO_TOML = """\
[{name}]
name = "{display_name}"
description = "{display_name} MCP server"
transport = "stdio"
command = "/usr/local/bin/{name}"
enabled = true
"""

_MCP_SSE_TOML = """\
[{name}]
name = "{display_name}"
description = "{display_name} MCP server"
transport = "sse"
url = "http://localhost:8080/sse"
enabled = true
"""


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def init_guest_dir(target: Path, *, force: bool = False) -> None:
    """Create a new guest config directory with minimal scaffolding.

    Creates:
        target/config/build.toml
        target/config/kernel/defconfig.arm64

    Args:
        target: Directory to create (e.g., ./guest).
        force: If True, overwrite existing config/ directory.

    Raises:
        FileExistsError: If target/config already exists and force is False.
    """
    config_dir = target / "config"
    if config_dir.exists() and not force:
        raise FileExistsError(f"{config_dir} already exists (use --force to overwrite)")

    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "build.toml").write_text(_BUILD_TOML)

    kernel_dir = config_dir / "kernel"
    kernel_dir.mkdir(exist_ok=True)
    (kernel_dir / "defconfig.arm64").write_text(_DEFCONFIG_STUB)


def add_ai_provider(
    guest_dir: Path,
    name: str,
    *,
    force: bool = False,
) -> Path:
    """Add an AI provider TOML template.

    Args:
        guest_dir: Guest directory (contains config/).
        name: Provider key (e.g., "openai", "mistral").
        force: Overwrite if file exists.

    Returns:
        Path to the created TOML file.

    Raises:
        FileExistsError: If the provider file already exists and force is False.
        FileNotFoundError: If guest_dir/config doesn't exist.
    """
    config_dir = guest_dir / "config"
    if not config_dir.is_dir():
        raise FileNotFoundError(f"{config_dir} not found (run 'init' first)")

    ai_dir = config_dir / "ai"
    ai_dir.mkdir(exist_ok=True)

    path = ai_dir / f"{name}.toml"
    if path.exists() and not force:
        raise FileExistsError(f"{path} already exists (use --force to overwrite)")

    display_name = name.replace("_", " ").replace("-", " ").title()
    env_var = f"{name.upper()}_API_KEY"
    content = _AI_PROVIDER_TOML.format(
        name=name, display_name=display_name, env_var=env_var
    )
    path.write_text(content)
    return path


def add_package_set(
    guest_dir: Path,
    name: str,
    *,
    manager: str = "apt",
    force: bool = False,
) -> Path:
    """Add a package set TOML template.

    Args:
        guest_dir: Guest directory (contains config/).
        name: Package set key (e.g., "system", "python").
        manager: Package manager (apt, uv, pip, npm).
        force: Overwrite if file exists.

    Returns:
        Path to the created TOML file.

    Raises:
        FileExistsError: If the file already exists and force is False.
        FileNotFoundError: If guest_dir/config doesn't exist.
    """
    config_dir = guest_dir / "config"
    if not config_dir.is_dir():
        raise FileNotFoundError(f"{config_dir} not found (run 'init' first)")

    pkg_dir = config_dir / "packages"
    pkg_dir.mkdir(exist_ok=True)

    path = pkg_dir / f"{name}.toml"
    if path.exists() and not force:
        raise FileExistsError(f"{path} already exists (use --force to overwrite)")

    display_name = name.replace("_", " ").replace("-", " ").title()
    install_cmd = _INSTALL_CMDS.get(manager, _INSTALL_CMDS["apt"])
    content = _PACKAGE_SET_TOML.format(
        name=name, display_name=display_name, manager=manager, install_cmd=install_cmd
    )
    path.write_text(content)
    return path


def add_mcp_server(
    guest_dir: Path,
    name: str,
    *,
    transport: str = "stdio",
    force: bool = False,
) -> Path:
    """Add an MCP server TOML template.

    Args:
        guest_dir: Guest directory (contains config/).
        name: Server key (e.g., "myserver").
        transport: Transport type ("stdio" or "sse").
        force: Overwrite if file exists.

    Returns:
        Path to the created TOML file.

    Raises:
        FileExistsError: If the file already exists and force is False.
        FileNotFoundError: If guest_dir/config doesn't exist.
    """
    config_dir = guest_dir / "config"
    if not config_dir.is_dir():
        raise FileNotFoundError(f"{config_dir} not found (run 'init' first)")

    mcp_dir = config_dir / "mcp"
    mcp_dir.mkdir(exist_ok=True)

    path = mcp_dir / f"{name}.toml"
    if path.exists() and not force:
        raise FileExistsError(f"{path} already exists (use --force to overwrite)")

    display_name = name.replace("_", " ").replace("-", " ").title()
    if transport == "sse":
        content = _MCP_SSE_TOML.format(name=name, display_name=display_name)
    else:
        content = _MCP_STDIO_TOML.format(name=name, display_name=display_name)
    path.write_text(content)
    return path
