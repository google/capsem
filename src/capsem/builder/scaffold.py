"""Scaffolding for guest image configurations.

Creates directory structures and template TOML files for new guest images,
AI providers, package sets, and MCP servers. The `new_image` function creates
a new image directory by selecting components from a base config.
"""

from __future__ import annotations

import shutil
from datetime import date
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]


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


# ---------------------------------------------------------------------------
# Scan base config
# ---------------------------------------------------------------------------


def _parse_toml_safe(path: Path) -> dict:
    """Parse a TOML file, returning empty dict on error."""
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except Exception:
        return {}


def scan_base_config(base_dir: Path) -> dict:
    """Scan a base config directory and return available components.

    Returns dict with keys: providers, packages, mcp, has_security, has_vm.
    Each component dict maps key -> display description.
    """
    config_dir = base_dir / "config"
    result: dict = {
        "providers": {},
        "packages": {},
        "mcp": {},
        "has_security": False,
        "has_vm": False,
    }

    # AI providers
    ai_dir = config_dir / "ai"
    if ai_dir.is_dir():
        for path in sorted(ai_dir.glob("*.toml")):
            data = _parse_toml_safe(path)
            key = path.stem
            for section in data.values():
                if isinstance(section, dict) and "name" in section:
                    desc = section.get("description", section["name"])
                    result["providers"][key] = f"{section['name']} -- {desc}"
                    break

    # Package sets
    pkg_dir = config_dir / "packages"
    if pkg_dir.is_dir():
        for path in sorted(pkg_dir.glob("*.toml")):
            data = _parse_toml_safe(path)
            key = path.stem
            for section in data.values():
                if isinstance(section, dict) and "name" in section:
                    pkgs = section.get("packages", [])
                    count = len(pkgs)
                    result["packages"][key] = (
                        f"{section['name']} ({count} package{'s' if count != 1 else ''})"
                    )
                    break

    # MCP servers
    mcp_dir = config_dir / "mcp"
    if mcp_dir.is_dir():
        for path in sorted(mcp_dir.glob("*.toml")):
            data = _parse_toml_safe(path)
            key = path.stem
            for section in data.values():
                if isinstance(section, dict) and "name" in section:
                    desc = section.get("description", section["name"])
                    result["mcp"][key] = desc
                    break

    # Security and VM
    result["has_security"] = (config_dir / "security" / "web.toml").is_file()
    result["has_vm"] = (config_dir / "vm").is_dir() and any(
        (config_dir / "vm").glob("*.toml")
    )

    return result


# ---------------------------------------------------------------------------
# Create new image from base
# ---------------------------------------------------------------------------


_MANIFEST_TOML = """\
[image]
name = "{name}"
version = "{version}"
description = "{description}"

[[image.changelog]]
version = "{version}"
date = "{today}"
changes = ["Initial image created from {base_name}"]
"""


def new_image(
    target: Path,
    base_dir: Path,
    *,
    name: str | None = None,
    version: str = "0.1.0",
    description: str = "",
    include_providers: list[str] | None = None,
    include_packages: list[str] | None = None,
    include_mcp: list[str] | None = None,
    include_security: bool = True,
    include_vm: bool = True,
    force: bool = False,
) -> Path:
    """Create a new image directory by selecting components from a base config.

    Args:
        target: Directory to create (e.g., ./corp-image).
        base_dir: Base config to copy from (e.g., ./guest).
        name: Image name (defaults to target directory name).
        version: Image version.
        description: One-line description.
        include_providers: Provider keys to include (None = all).
        include_packages: Package set keys to include (None = all).
        include_mcp: MCP server keys to include (None = all).
        include_security: Copy security/ config.
        include_vm: Copy vm/ config.
        force: Overwrite existing config dir.

    Returns:
        Path to the created config directory.
    """
    config_dir = target / "config"
    if config_dir.exists() and not force:
        raise FileExistsError(f"{config_dir} already exists (use --force to overwrite)")

    base_config = base_dir / "config"
    if name is None:
        name = target.name

    # Create target config dir
    config_dir.mkdir(parents=True, exist_ok=True)

    # Always copy build.toml
    shutil.copy2(str(base_config / "build.toml"), str(config_dir / "build.toml"))

    # Always copy kernel defconfigs
    kernel_src = base_config / "kernel"
    if kernel_src.is_dir():
        kernel_dst = config_dir / "kernel"
        kernel_dst.mkdir(exist_ok=True)
        for f in kernel_src.glob("defconfig.*"):
            shutil.copy2(str(f), str(kernel_dst / f.name))

    # AI providers
    ai_src = base_config / "ai"
    if ai_src.is_dir():
        available = [p.stem for p in sorted(ai_src.glob("*.toml"))]
        selected = available if include_providers is None else include_providers
        if selected:
            ai_dst = config_dir / "ai"
            ai_dst.mkdir(exist_ok=True)
            for key in selected:
                src = ai_src / f"{key}.toml"
                if src.is_file():
                    shutil.copy2(str(src), str(ai_dst / f"{key}.toml"))

    # Package sets
    pkg_src = base_config / "packages"
    if pkg_src.is_dir():
        available = [p.stem for p in sorted(pkg_src.glob("*.toml"))]
        selected = available if include_packages is None else include_packages
        if selected:
            pkg_dst = config_dir / "packages"
            pkg_dst.mkdir(exist_ok=True)
            for key in selected:
                src = pkg_src / f"{key}.toml"
                if src.is_file():
                    shutil.copy2(str(src), str(pkg_dst / f"{key}.toml"))

    # MCP servers
    mcp_src = base_config / "mcp"
    if mcp_src.is_dir():
        available = [p.stem for p in sorted(mcp_src.glob("*.toml"))]
        selected = available if include_mcp is None else include_mcp
        if selected:
            mcp_dst = config_dir / "mcp"
            mcp_dst.mkdir(exist_ok=True)
            for key in selected:
                src = mcp_src / f"{key}.toml"
                if src.is_file():
                    shutil.copy2(str(src), str(mcp_dst / f"{key}.toml"))

    # Security
    if include_security:
        sec_src = base_config / "security"
        if sec_src.is_dir():
            sec_dst = config_dir / "security"
            sec_dst.mkdir(exist_ok=True)
            for f in sec_src.glob("*.toml"):
                shutil.copy2(str(f), str(sec_dst / f.name))

    # VM config
    if include_vm:
        vm_src = base_config / "vm"
        if vm_src.is_dir():
            vm_dst = config_dir / "vm"
            vm_dst.mkdir(exist_ok=True)
            for f in vm_src.glob("*.toml"):
                shutil.copy2(str(f), str(vm_dst / f.name))

    # Generate manifest.toml
    base_name = base_dir.name
    manifest_content = _MANIFEST_TOML.format(
        name=name,
        version=version,
        description=description,
        today=date.today().isoformat(),
        base_name=base_name,
    )
    (config_dir / "manifest.toml").write_text(manifest_content)

    return config_dir
