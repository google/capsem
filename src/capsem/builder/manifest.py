"""BOM manifest models, parsers, and renderer.

Pydantic models for the bill-of-materials manifest. Parsers accept
pre-captured command output strings (dpkg-query, pip list, npm ls, b3sum).
render() produces a plain-text table -- the single human-readable output path.
"""

from __future__ import annotations

import json

from pydantic import BaseModel, ConfigDict, Field, model_validator


# ---------------------------------------------------------------------------
# Models
# ---------------------------------------------------------------------------


class PackageEntry(BaseModel):
    """A single installed package from dpkg, pip, or npm."""

    model_config = ConfigDict(frozen=True)

    name: str
    version: str
    source: str  # "dpkg", "pip", "npm"
    arch: str = ""


class AssetEntry(BaseModel):
    """A build artifact with its BLAKE3 hash."""

    model_config = ConfigDict(frozen=True)

    filename: str
    hash: str = Field(min_length=64, max_length=64)
    size: int = Field(ge=0)


class VulnEntry(BaseModel):
    """A single vulnerability finding from trivy or grype."""

    model_config = ConfigDict(frozen=True)

    id: str
    severity: str
    package: str
    installed_version: str
    fixed_version: str = ""
    title: str = ""
    scanner: str = ""


class ArchManifest(BaseModel):
    """Per-architecture manifest combining packages, assets, vulns."""

    model_config = ConfigDict(frozen=True)

    arch: str
    packages: list[PackageEntry] = Field(default_factory=list)
    assets: list[AssetEntry] = Field(default_factory=list)
    vulns: list[VulnEntry] = Field(default_factory=list)

    @model_validator(mode="after")
    def _arch_non_empty(self):
        if not self.arch:
            raise ValueError("arch must not be empty")
        return self


class ImageManifest(BaseModel):
    """Top-level manifest combining all architectures."""

    model_config = ConfigDict(frozen=True)

    version: str
    created: str
    architectures: dict[str, ArchManifest] = Field(default_factory=dict)

    @model_validator(mode="after")
    def _version_non_empty(self):
        if not self.version:
            raise ValueError("version must not be empty")
        return self


# ---------------------------------------------------------------------------
# Parsers
# ---------------------------------------------------------------------------


def parse_dpkg_query(output: str) -> list[PackageEntry]:
    """Parse dpkg-query -W -f='${Package}\\t${Version}\\t${Architecture}\\n' output."""
    entries: list[PackageEntry] = []
    for line in output.strip().splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        entries.append(PackageEntry(
            name=parts[0], version=parts[1], source="dpkg", arch=parts[2],
        ))
    return sorted(entries, key=lambda p: p.name)


def parse_pip_list(output: str) -> list[PackageEntry]:
    """Parse pip list --format json output."""
    try:
        data = json.loads(output)
    except json.JSONDecodeError as e:
        raise ValueError(f"Invalid pip list JSON: {e}") from e
    entries: list[PackageEntry] = []
    for item in data:
        name = item.get("name", "")
        version = item.get("version", "")
        if name and version:
            entries.append(PackageEntry(name=name, version=version, source="pip"))
    return sorted(entries, key=lambda p: p.name)


def parse_npm_ls(output: str) -> list[PackageEntry]:
    """Parse npm ls --json --global output."""
    try:
        data = json.loads(output)
    except json.JSONDecodeError as e:
        raise ValueError(f"Invalid npm ls JSON: {e}") from e
    deps = data.get("dependencies", {})
    entries: list[PackageEntry] = []
    for name, info in deps.items():
        version = info.get("version", "") if isinstance(info, dict) else ""
        if name and version:
            entries.append(PackageEntry(name=name, version=version, source="npm"))
    return sorted(entries, key=lambda p: p.name)


def parse_b3sums(
    output: str,
    *,
    sizes: dict[str, int] | None = None,
) -> list[AssetEntry]:
    """Parse b3sum output (hash  filename per line)."""
    entries: list[AssetEntry] = []
    for line in output.strip().splitlines():
        parts = line.split(None, 1)
        if len(parts) < 2:
            continue
        h, filename = parts[0], parts[1].strip()
        size = (sizes or {}).get(filename, 0)
        entries.append(AssetEntry(filename=filename, hash=h, size=size))
    return entries


def collect_bom(
    *,
    arch: str,
    dpkg_output: str = "",
    pip_output: str = "[]",
    npm_output: str = "{}",
    b3sum_output: str = "",
    sizes: dict[str, int] | None = None,
) -> ArchManifest:
    """Collect BOM from all parser outputs into an ArchManifest."""
    packages: list[PackageEntry] = []
    packages.extend(parse_dpkg_query(dpkg_output))
    packages.extend(parse_pip_list(pip_output))
    packages.extend(parse_npm_ls(npm_output))
    assets = parse_b3sums(b3sum_output, sizes=sizes)
    return ArchManifest(arch=arch, packages=packages, assets=assets)


# ---------------------------------------------------------------------------
# Renderer
# ---------------------------------------------------------------------------


def _human_size(size: int) -> str:
    """Format bytes as human-readable size."""
    if size >= 1_000_000_000:
        return f"{size / 1_000_000_000:.1f} GB"
    if size >= 1_000_000:
        return f"{size / 1_000_000:.1f} MB"
    if size >= 1_000:
        return f"{size / 1_000:.1f} KB"
    return f"{size} B"


def _table(headers: list[str], rows: list[list[str]]) -> str:
    """Render a plain-text aligned table."""
    if not rows:
        return ""
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            if i < len(widths):
                widths[i] = max(widths[i], len(cell))
    lines: list[str] = []
    header_line = "  ".join(h.ljust(widths[i]) for i, h in enumerate(headers))
    lines.append(f"  {header_line}")
    for row in rows:
        cells = "  ".join(
            (row[i] if i < len(row) else "").ljust(widths[i])
            for i in range(len(headers))
        )
        lines.append(f"  {cells}")
    return "\n".join(lines)


def render(manifest: ImageManifest) -> str:
    """Render an ImageManifest as plain text tables."""
    parts: list[str] = []
    parts.append(f"Image Manifest v{manifest.version}  ({manifest.created})")

    if not manifest.architectures:
        parts.append("\nNo architectures.")
        return "\n".join(parts)

    for arch_name, arch in manifest.architectures.items():
        parts.append(f"\n=== {arch_name} ===")

        # Group packages by source
        by_source: dict[str, list[PackageEntry]] = {}
        for pkg in arch.packages:
            by_source.setdefault(pkg.source, []).append(pkg)

        for source in ("dpkg", "pip", "npm"):
            pkgs = by_source.get(source, [])
            if not pkgs:
                continue
            parts.append(f"\nPackages ({source}): {len(pkgs)}")
            if source == "dpkg":
                rows = [[p.name, p.version, p.arch] for p in pkgs]
                parts.append(_table(["PACKAGE", "VERSION", "ARCH"], rows))
            else:
                rows = [[p.name, p.version] for p in pkgs]
                parts.append(_table(["PACKAGE", "VERSION"], rows))

        if arch.assets:
            parts.append(f"\nAssets: {len(arch.assets)}")
            rows = [[a.filename, _human_size(a.size), a.hash[:16] + "..."]
                    for a in arch.assets]
            parts.append(_table(["FILENAME", "SIZE", "B3 HASH"], rows))

        if arch.vulns:
            parts.append(f"\nVulnerabilities: {len(arch.vulns)}")
            rows = [[v.id, v.severity, v.package, v.installed_version, v.fixed_version]
                    for v in arch.vulns]
            parts.append(_table(
                ["ID", "SEVERITY", "PACKAGE", "INSTALLED", "FIXED"], rows,
            ))

    return "\n".join(parts)
