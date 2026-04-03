"""Compiler-style config linter for guest image configurations.

Validates TOML configs, generated JSON, artifacts, and cross-language
conformance. Returns a list of Diagnostic objects with error codes,
severity, file paths, and optional line numbers.

Error code ranges:
  E001-E010: TOML config validation
  E100-E103: Schema / generated JSON validation
  E200-E202: Cross-language conformance
  E300-E305: Artifact validation
  E400-E402: Docker validation
  W001-W013: Warnings
"""

from __future__ import annotations

import json
import re
import tomllib
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any

from pydantic import ValidationError

from capsem.builder.config import load_guest_config
from capsem.builder.models import GuestImageConfig


# ---------------------------------------------------------------------------
# Core types
# ---------------------------------------------------------------------------


class Severity(str, Enum):
    """Diagnostic severity level."""

    ERROR = "error"
    WARNING = "warning"


@dataclass
class Diagnostic:
    """A single compiler-style diagnostic."""

    code: str
    severity: Severity
    message: str
    file: str
    line: int | None = None

    def __str__(self) -> str:
        loc = self.file
        if self.line is not None:
            loc = f"{self.file}:{self.line}"
        return f"{self.severity.value}: [{self.code}] {loc}: {self.message}"


# ---------------------------------------------------------------------------
# TOML line tracking
# ---------------------------------------------------------------------------


def find_toml_line(text: str, key: str) -> int | None:
    """Find the line number (1-based) of a TOML key or section header.

    Searches for [key] as a section header or key = as an assignment.
    Returns None if not found.
    """
    # Try section header first: [key] or [key.sub.path]
    escaped = re.escape(key)
    section_pat = re.compile(rf"^\[{escaped}\]", re.MULTILINE)
    m = section_pat.search(text)
    if m:
        return text[:m.start()].count("\n") + 1

    # Try key assignment: key = or last segment of dotted key
    last_segment = key.rsplit(".", 1)[-1]
    assign_pat = re.compile(rf"^{re.escape(last_segment)}\s*=", re.MULTILINE)
    m = assign_pat.search(text)
    if m:
        return text[:m.start()].count("\n") + 1

    return None


# ---------------------------------------------------------------------------
# Secret detection patterns
# ---------------------------------------------------------------------------

_SECRET_PATTERNS = [
    re.compile(r"sk-ant-[a-zA-Z0-9_-]{10,}"),  # Anthropic
    re.compile(r"AIza[a-zA-Z0-9_-]{10,}"),  # Google
    re.compile(r"sk-[a-zA-Z0-9]{20,}"),  # OpenAI
    re.compile(r"ghp_[a-zA-Z0-9]{20,}"),  # GitHub PAT
    re.compile(r"glpat-[a-zA-Z0-9_-]{10,}"),  # GitLab PAT
    re.compile(r"Bearer\s+[a-zA-Z0-9_.-]{20,}"),  # Bearer tokens
]

_PLACEHOLDER_PATTERNS = [
    re.compile(r"^TODO$", re.IGNORECASE),
    re.compile(r"^FIXME$", re.IGNORECASE),
    re.compile(r"^CHANGEME$", re.IGNORECASE),
    re.compile(r"^REPLACE_ME$", re.IGNORECASE),
    re.compile(r"^XXX$", re.IGNORECASE),
]

_DOMAIN_BAD_PATTERNS = [
    re.compile(r"^https?://"),  # Has scheme
    re.compile(r"/"),  # Has path
    re.compile(r":\d+"),  # Has port
]


# ---------------------------------------------------------------------------
# Validators
# ---------------------------------------------------------------------------


def _validate_toml_files(
    config_dir: Path,
    diags: list[Diagnostic],
) -> dict[str, dict[str, Any]] | None:
    """Parse all TOML files, emit E001/E002. Returns parsed data or None."""
    build_path = config_dir / "build.toml"
    if not build_path.exists():
        diags.append(Diagnostic(
            code="E001",
            severity=Severity.ERROR,
            message="Missing required file: build.toml",
            file=str(build_path),
        ))
        return None

    parsed: dict[str, dict[str, Any]] = {}
    toml_files = list(config_dir.rglob("*.toml"))

    for path in sorted(toml_files):
        try:
            with open(path, "rb") as f:
                data = tomllib.load(f)
            parsed[str(path)] = data
        except tomllib.TOMLDecodeError as e:
            diags.append(Diagnostic(
                code="E002",
                severity=Severity.ERROR,
                message=f"Invalid TOML syntax: {e}",
                file=str(path.relative_to(config_dir.parent)),
            ))

    return parsed


def _validate_pydantic(
    guest_dir: Path,
    diags: list[Diagnostic],
) -> GuestImageConfig | None:
    """Load and validate config through Pydantic, emit E003-E005."""
    try:
        config = load_guest_config(guest_dir)
        return config
    except FileNotFoundError:
        # Already caught by E001
        return None
    except tomllib.TOMLDecodeError:
        # Already caught by E002
        return None
    except ValidationError as e:
        config_dir = guest_dir / "config"
        for err in e.errors():
            loc = ".".join(str(p) for p in err["loc"])
            msg = err["msg"]

            # Determine specific error code
            code = "E003"
            if "packages" in msg.lower() and ("empty" in msg.lower() or "at least one" in msg.lower()):
                code = "E004"
            elif "input should be" in msg.lower() and ("manager" in loc.lower() or "manager" in msg.lower()):
                code = "E005"

            # Try to find the file and line
            file_str = _guess_file_for_error(config_dir, loc)

            diags.append(Diagnostic(
                code=code,
                severity=Severity.ERROR,
                message=f"{loc}: {msg}",
                file=file_str,
            ))
        return None


def _guess_file_for_error(config_dir: Path, loc: str) -> str:
    """Best-effort file path for a Pydantic validation error location."""
    loc_lower = loc.lower()
    if "ai_provider" in loc_lower:
        return "config/ai/*.toml"
    if "package_set" in loc_lower:
        return "config/packages/*.toml"
    if "mcp_server" in loc_lower:
        return "config/mcp/*.toml"
    if "web_security" in loc_lower:
        return "config/security/web.toml"
    if "vm_resource" in loc_lower:
        return "config/vm/resources.toml"
    if "vm_environment" in loc_lower:
        return "config/vm/environment.toml"
    return "config/build.toml"


def _validate_domains(config: GuestImageConfig, diags: list[Diagnostic]) -> None:
    """Validate domain patterns, emit E006."""
    # AI provider domains
    for key, prov in config.ai_providers.items():
        for domain in prov.network.domains:
            if _is_bad_domain(domain):
                diags.append(Diagnostic(
                    code="E006",
                    severity=Severity.ERROR,
                    message=f"Invalid domain pattern '{domain}' in ai.{key}.network.domains",
                    file=f"config/ai/{key}.toml",
                ))

    # Web security domains
    ws = config.web_security
    for section_name, section in [("search", ws.search), ("registry", ws.registry), ("repository", ws.repository)]:
        for key, svc in section.items():
            for domain in svc.domains:
                if _is_bad_domain(domain):
                    diags.append(Diagnostic(
                        code="E006",
                        severity=Severity.ERROR,
                        message=f"Invalid domain pattern '{domain}' in web.{section_name}.{key}.domains",
                        file="config/security/web.toml",
                    ))


def _is_bad_domain(domain: str) -> bool:
    """Check if a domain pattern is malformed."""
    if not domain or domain.isspace():
        return True
    for pat in _DOMAIN_BAD_PATTERNS:
        if pat.search(domain):
            return True
    return False


def _validate_file_paths(config: GuestImageConfig, diags: list[Diagnostic]) -> None:
    """Validate file paths are absolute and JSON content is valid, emit E009/E010."""
    for key, prov in config.ai_providers.items():
        for file_key, file_cfg in prov.files.items():
            # E009: File path must be absolute
            if not file_cfg.path.startswith("/"):
                diags.append(Diagnostic(
                    code="E009",
                    severity=Severity.ERROR,
                    message=f"File path must be absolute: '{file_cfg.path}' in ai.{key}.files.{file_key}",
                    file=f"config/ai/{key}.toml",
                ))
            # E010: JSON files must have valid JSON content
            if file_cfg.path.endswith(".json") and file_cfg.content:
                try:
                    json.loads(file_cfg.content)
                except json.JSONDecodeError as e:
                    diags.append(Diagnostic(
                        code="E010",
                        severity=Severity.ERROR,
                        message=f"Invalid JSON in ai.{key}.files.{file_key}: {e}",
                        file=f"config/ai/{key}.toml",
                    ))


def _validate_duplicates(
    config_dir: Path,
    parsed: dict[str, dict[str, Any]],
    diags: list[Diagnostic],
) -> None:
    """Check for duplicate keys across files in the same directory, emit E008."""
    # Check AI providers
    ai_dir = config_dir / "ai"
    if ai_dir.is_dir():
        _check_dir_key_collisions(ai_dir, parsed, diags, "AI provider")

    # Check MCP servers
    mcp_dir = config_dir / "mcp"
    if mcp_dir.is_dir():
        _check_dir_key_collisions(mcp_dir, parsed, diags, "MCP server")

    # Check package sets
    pkg_dir = config_dir / "packages"
    if pkg_dir.is_dir():
        _check_dir_key_collisions(pkg_dir, parsed, diags, "package set")


def _check_dir_key_collisions(
    directory: Path,
    parsed: dict[str, dict[str, Any]],
    diags: list[Diagnostic],
    label: str,
) -> None:
    """Check for duplicate top-level keys across TOML files in a directory."""
    seen: dict[str, str] = {}  # key -> first file
    for path in sorted(directory.glob("*.toml")):
        path_str = str(path)
        if path_str not in parsed:
            continue
        data = parsed[path_str]
        for key in data:
            if key in seen:
                diags.append(Diagnostic(
                    code="E008",
                    severity=Severity.ERROR,
                    message=f"Duplicate {label} key '{key}' (first defined in {Path(seen[key]).name})",
                    file=str(path.relative_to(directory.parent.parent)),
                ))
            else:
                seen[key] = path_str


def _validate_defconfigs(
    config: GuestImageConfig,
    config_dir: Path,
    diags: list[Diagnostic],
) -> None:
    """Check that defconfig files exist for each architecture, emit E300."""
    kernel_dir = config_dir / "kernel"
    for arch_name, arch in config.build.architectures.items():
        defconfig_path = kernel_dir / Path(arch.defconfig).name
        if not defconfig_path.exists():
            diags.append(Diagnostic(
                code="E300",
                severity=Severity.ERROR,
                message=f"Missing kernel defconfig for {arch_name}: {arch.defconfig}",
                file=str(defconfig_path),
            ))


def _validate_artifacts(
    artifacts_dir: Path,
    diags: list[Diagnostic],
) -> None:
    """Check required artifacts exist, emit E301/E302.

    Uses the canonical lists from docker.py so there is one place to add
    a new artifact.
    """
    from capsem.builder.docker import (
        ROOTFS_SCRIPTS,
        ROOTFS_SCRIPT_DIRS,
        ROOTFS_SUPPORT_FILES,
    )

    # E301: CA certificate
    ca_cert = artifacts_dir / "capsem-ca.crt"
    if not ca_cert.exists():
        diags.append(Diagnostic(
            code="E301",
            severity=Severity.ERROR,
            message="Missing CA certificate: capsem-ca.crt",
            file=str(ca_cert),
        ))

    # E302: Required files (single source of truth from docker.py)
    required_files = ["capsem-init"] + list(ROOTFS_SUPPORT_FILES) + list(ROOTFS_SCRIPTS)
    for name in required_files:
        path = artifacts_dir / name
        if not path.exists():
            diags.append(Diagnostic(
                code="E302",
                severity=Severity.ERROR,
                message=f"Missing required artifact: {name}",
                file=str(path),
            ))

    # Required directories
    for name in ROOTFS_SCRIPT_DIRS:
        path = artifacts_dir / name
        if not path.is_dir():
            diags.append(Diagnostic(
                code="E302",
                severity=Severity.ERROR,
                message=f"Missing required directory: {name}",
                file=str(path),
            ))


def _validate_warnings(
    config: GuestImageConfig,
    diags: list[Diagnostic],
) -> None:
    """Emit W001-W012 warnings."""
    ws = config.web_security

    # W001: Package set uses a registry but no registry configured in web security
    if config.package_sets and not ws.registry:
        diags.append(Diagnostic(
            code="W001",
            severity=Severity.WARNING,
            message="Package sets configured but no package registry in web security",
            file="config/security/web.toml",
        ))

    # W002: -dev packages in package lists
    for key, ps in config.package_sets.items():
        for pkg in ps.packages:
            if pkg.endswith("-dev") or pkg.endswith("-devel"):
                diags.append(Diagnostic(
                    code="W002",
                    severity=Severity.WARNING,
                    message=f"Package '{pkg}' looks like a development package in {key}",
                    file=f"config/packages/{key}.toml",
                ))

    # W003: Potential secrets in file content, MCP headers/env, shell configs
    for key, prov in config.ai_providers.items():
        for file_key, file_cfg in prov.files.items():
            if _contains_secret(file_cfg.content):
                diags.append(Diagnostic(
                    code="W003",
                    severity=Severity.WARNING,
                    message=f"Potential secret in ai.{key}.files.{file_key}.content",
                    file=f"config/ai/{key}.toml",
                ))
    for key, mcp in config.mcp_servers.items():
        for hdr_key, hdr_val in mcp.headers.items():
            if _contains_secret(hdr_val):
                diags.append(Diagnostic(
                    code="W003",
                    severity=Severity.WARNING,
                    message=f"Potential secret in mcp.{key}.headers.{hdr_key}",
                    file=f"config/mcp/{key}.toml",
                ))
        for env_key, env_val in mcp.env.items():
            if _contains_secret(env_val):
                diags.append(Diagnostic(
                    code="W003",
                    severity=Severity.WARNING,
                    message=f"Potential secret in mcp.{key}.env.{env_key}",
                    file=f"config/mcp/{key}.toml",
                ))
    # Also scan bashrc and tmux_conf content
    env = config.vm_environment
    if env.shell.bashrc and _contains_secret(env.shell.bashrc.content):
        diags.append(Diagnostic(
            code="W003",
            severity=Severity.WARNING,
            message="Potential secret in shell bashrc content",
            file="config/vm/environment.toml",
        ))
    if env.shell.tmux_conf and _contains_secret(env.shell.tmux_conf.content):
        diags.append(Diagnostic(
            code="W003",
            severity=Severity.WARNING,
            message="Potential secret in shell tmux_conf content",
            file="config/vm/environment.toml",
        ))

    # W004: Package set with no network config
    for key, ps in config.package_sets.items():
        if ps.network is None:
            diags.append(Diagnostic(
                code="W004",
                severity=Severity.WARNING,
                message=f"Package set '{key}' has no network config (can't download at build time)",
                file=f"config/packages/{key}.toml",
            ))

    # W005: Overlapping allow and block lists
    allow_set = set(ws.custom_allow)
    block_set = set(ws.custom_block)
    overlap = allow_set & block_set
    if overlap:
        diags.append(Diagnostic(
            code="W005",
            severity=Severity.WARNING,
            message=f"Domains in both allow and block lists: {', '.join(sorted(overlap))}",
            file="config/security/web.toml",
        ))

    # W006: Placeholder file content
    for key, prov in config.ai_providers.items():
        for file_key, file_cfg in prov.files.items():
            if _is_placeholder(file_cfg.content):
                diags.append(Diagnostic(
                    code="W006",
                    severity=Severity.WARNING,
                    message=f"File content looks like a placeholder in ai.{key}.files.{file_key}",
                    file=f"config/ai/{key}.toml",
                ))

    # W007: Overly broad wildcard domains
    _check_broad_wildcards(config, diags)

    # W008: Duplicate env_vars across AI providers
    _check_duplicate_env_vars(config, diags)

    # W009: Shell metacharacters in install_cmd
    for key, ps in config.package_sets.items():
        if _has_shell_metachar(ps.install_cmd):
            diags.append(Diagnostic(
                code="W009",
                severity=Severity.WARNING,
                message=f"Shell metacharacters in install_cmd for {key}",
                file=f"config/packages/{key}.toml",
            ))

    # W010: PATH missing essential directories
    path_dirs = set(env.shell.path.split(":"))
    if "/usr/bin" not in path_dirs and "/bin" not in path_dirs:
        diags.append(Diagnostic(
            code="W010",
            severity=Severity.WARNING,
            message="PATH is missing /usr/bin and /bin",
            file="config/vm/environment.toml",
        ))

    # W011: Wide-open network policy (both allow_read and allow_write, no block list)
    if ws.allow_read and ws.allow_write and not ws.custom_block:
        diags.append(Diagnostic(
            code="W011",
            severity=Severity.WARNING,
            message="Network policy is wide open: allow_read and allow_write both true with no block list",
            file="config/security/web.toml",
        ))

    # W012: Unknown rust_target (not a known musl target)
    _check_rust_targets(config, diags)


# Top-level domains that are too broad when used as *.tld wildcards
_BROAD_TLDS = {"com", "net", "org", "io", "dev", "co", "us", "uk", "eu", "app"}

_KNOWN_MUSL_TARGETS = {
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
}

_SHELL_METACHAR_PAT = re.compile(r"[;|&`$()]")


def _is_broad_wildcard(domain: str) -> bool:
    """Check if a domain is an overly broad wildcard like * or *.com."""
    if domain == "*":
        return True
    if domain.startswith("*."):
        suffix = domain[2:]
        # *.com, *.net, etc. -- just a TLD
        if suffix in _BROAD_TLDS:
            return True
        # *.co.uk etc.
        if "." not in suffix and len(suffix) <= 3:
            return True
    return False


def _check_broad_wildcards(config: GuestImageConfig, diags: list[Diagnostic]) -> None:
    """Emit W007 for overly broad wildcard domains."""
    # AI provider domains
    for key, prov in config.ai_providers.items():
        for domain in prov.network.domains:
            if _is_broad_wildcard(domain):
                diags.append(Diagnostic(
                    code="W007",
                    severity=Severity.WARNING,
                    message=f"Overly broad wildcard domain '{domain}' in ai.{key}",
                    file=f"config/ai/{key}.toml",
                ))
    # Web security custom_allow
    ws = config.web_security
    for domain in ws.custom_allow:
        if _is_broad_wildcard(domain):
            diags.append(Diagnostic(
                code="W007",
                severity=Severity.WARNING,
                message=f"Overly broad wildcard domain '{domain}' in custom_allow",
                file="config/security/web.toml",
            ))
    # Web security service domains
    for section_name, section in [("search", ws.search), ("registry", ws.registry), ("repository", ws.repository)]:
        for key, svc in section.items():
            for domain in svc.domains:
                if _is_broad_wildcard(domain):
                    diags.append(Diagnostic(
                        code="W007",
                        severity=Severity.WARNING,
                        message=f"Overly broad wildcard domain '{domain}' in web.{section_name}.{key}",
                        file="config/security/web.toml",
                    ))


def _check_duplicate_env_vars(config: GuestImageConfig, diags: list[Diagnostic]) -> None:
    """Emit W008 for duplicate env_vars across AI providers."""
    seen: dict[str, str] = {}  # env_var -> first provider key
    for key, prov in config.ai_providers.items():
        for var in prov.api_key.env_vars:
            if var in seen:
                diags.append(Diagnostic(
                    code="W008",
                    severity=Severity.WARNING,
                    message=f"Duplicate env_var '{var}' in ai.{key} (also in ai.{seen[var]})",
                    file=f"config/ai/{key}.toml",
                ))
            else:
                seen[var] = key


def _has_shell_metachar(cmd: str) -> bool:
    """Check if a command string contains shell metacharacters."""
    return bool(_SHELL_METACHAR_PAT.search(cmd))


def _check_rust_targets(config: GuestImageConfig, diags: list[Diagnostic]) -> None:
    """Emit W012 for unknown rust_target values."""
    for arch_name, arch in config.build.architectures.items():
        if arch.rust_target not in _KNOWN_MUSL_TARGETS:
            diags.append(Diagnostic(
                code="W012",
                severity=Severity.WARNING,
                message=f"Unknown rust_target '{arch.rust_target}' for {arch_name} (expected musl target)",
                file="config/build.toml",
            ))


def _check_ai_version_commands(
    config: GuestImageConfig, diags: list[Diagnostic],
) -> None:
    """Emit W013 for enabled AI providers with cli but no version_command."""
    for key, provider in config.ai_providers.items():
        if provider.enabled and provider.cli and not provider.cli.version_command:
            diags.append(Diagnostic(
                code="W013",
                severity=Severity.WARNING,
                message=(
                    f"AI provider '{key}' has cli but no version_command -- "
                    "tool-versions.txt will not track this CLI"
                ),
                file=f"config/ai/{key}.toml",
            ))


def _contains_secret(text: str) -> bool:
    """Check if text contains patterns that look like real secrets."""
    for pat in _SECRET_PATTERNS:
        if pat.search(text):
            return True
    return False


def _is_placeholder(content: str) -> bool:
    """Check if content looks like a placeholder."""
    stripped = content.strip()
    if not stripped:
        return False
    for pat in _PLACEHOLDER_PATTERNS:
        if pat.match(stripped):
            return True
    return False


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def validate_guest(
    guest_dir: Path,
    *,
    artifacts_dir: Path | None = None,
) -> list[Diagnostic]:
    """Validate a guest image configuration directory.

    Runs all validators and returns a list of diagnostics sorted by severity
    (errors first) then code.

    Args:
        guest_dir: Path to the guest directory (contains config/ subdirectory).
        artifacts_dir: Optional path to artifacts directory. If provided,
            checks for required artifacts (capsem-init, capsem-ca.crt, etc.).

    Returns:
        List of Diagnostic objects. Empty list means the config is clean.
    """
    diags: list[Diagnostic] = []
    config_dir = guest_dir / "config"

    # E001/E002: Parse TOML files
    if not config_dir.is_dir():
        diags.append(Diagnostic(
            code="E001",
            severity=Severity.ERROR,
            message="Missing config directory",
            file=str(config_dir),
        ))
        return sorted(diags, key=lambda d: (d.severity.value, d.code))

    parsed = _validate_toml_files(config_dir, diags)

    # If we can't parse TOML, skip deeper validation
    if parsed is None or any(d.code == "E001" for d in diags):
        return sorted(diags, key=lambda d: (d.severity.value, d.code))

    # E003-E005: Pydantic validation
    config = _validate_pydantic(guest_dir, diags)

    if config is None:
        return sorted(diags, key=lambda d: (d.severity.value, d.code))

    # E006: Domain validation
    _validate_domains(config, diags)

    # E008: Duplicate keys
    _validate_duplicates(config_dir, parsed, diags)

    # E009/E010: File path and content validation
    _validate_file_paths(config, diags)

    # E300: Defconfig validation
    _validate_defconfigs(config, config_dir, diags)

    # E301/E302: Artifact validation (optional)
    if artifacts_dir is not None:
        _validate_artifacts(artifacts_dir, diags)

    # W001-W006: Warnings
    _validate_warnings(config, diags)

    # W013: AI providers missing version_command
    _check_ai_version_commands(config, diags)

    return sorted(diags, key=lambda d: (d.severity.value, d.code))
