"""Tests for capsem.builder.manifest -- BOM models, parsers, and renderer.

TDD: tests written first (RED), then manifest.py makes them pass (GREEN).
"""

from __future__ import annotations

import json

import pytest
from pydantic import ValidationError

from capsem.builder.manifest import (
    ArchManifest,
    AssetEntry,
    ImageManifest,
    PackageEntry,
    VulnEntry,
    collect_bom,
    parse_b3sums,
    parse_dpkg_query,
    parse_npm_ls,
    parse_pip_list,
    render,
)

# ---------------------------------------------------------------------------
# Inline fixtures
# ---------------------------------------------------------------------------

DPKG_OUTPUT = """\
bash\t5.2.15-2+b2\tarm64
coreutils\t9.1-1\tarm64
curl\t7.88.1-10+deb12u8\tarm64
git\t1:2.39.5-0+deb12u1\tall
vim-tiny\t2:9.0.1378-2\tarm64
"""

DPKG_SINGLE = "bash\t5.2.15\tarm64\n"

PIP_JSON = json.dumps([
    {"name": "pytest", "version": "8.3.4"},
    {"name": "requests", "version": "2.31.0"},
    {"name": "httpx", "version": "0.27.0"},
])

PIP_EMPTY = "[]"

NPM_JSON = json.dumps({
    "dependencies": {
        "@anthropic-ai/claude-code": {"version": "1.0.36"},
        "@google/gemini-cli": {"version": "0.1.20"},
    }
})

NPM_EMPTY = json.dumps({"dependencies": {}})

NPM_NO_DEPS_KEY = json.dumps({"name": "root"})

B3SUMS_OUTPUT = """\
a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  vmlinuz
b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3  initrd.img
c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4  rootfs.squashfs
"""

SIZES = {"vmlinuz": 12_345_678, "initrd.img": 5_678_901, "rootfs.squashfs": 999_999_999}


# ---------------------------------------------------------------------------
# PackageEntry
# ---------------------------------------------------------------------------


class TestPackageEntry:

    def test_construction(self):
        p = PackageEntry(name="bash", version="5.2.15", source="dpkg", arch="arm64")
        assert p.name == "bash"
        assert p.version == "5.2.15"
        assert p.source == "dpkg"
        assert p.arch == "arm64"

    def test_defaults(self):
        p = PackageEntry(name="pytest", version="8.0", source="pip")
        assert p.arch == ""

    def test_frozen(self):
        p = PackageEntry(name="bash", version="5.0", source="dpkg")
        with pytest.raises(ValidationError):
            p.name = "zsh"

    def test_roundtrip(self):
        p = PackageEntry(name="curl", version="7.88", source="dpkg", arch="arm64")
        d = p.model_dump()
        p2 = PackageEntry.model_validate(d)
        assert p == p2


# ---------------------------------------------------------------------------
# AssetEntry
# ---------------------------------------------------------------------------


class TestAssetEntry:

    def test_construction(self):
        h = "a" * 64
        a = AssetEntry(filename="vmlinuz", hash=h, size=12345)
        assert a.filename == "vmlinuz"
        assert a.hash == h
        assert a.size == 12345

    def test_hash_too_short(self):
        with pytest.raises(ValidationError, match="String should have at least 64"):
            AssetEntry(filename="x", hash="abc", size=0)

    def test_hash_too_long(self):
        with pytest.raises(ValidationError, match="String should have at most 64"):
            AssetEntry(filename="x", hash="a" * 65, size=0)

    def test_negative_size(self):
        with pytest.raises(ValidationError):
            AssetEntry(filename="x", hash="a" * 64, size=-1)

    def test_zero_size_ok(self):
        a = AssetEntry(filename="x", hash="a" * 64, size=0)
        assert a.size == 0


# ---------------------------------------------------------------------------
# VulnEntry
# ---------------------------------------------------------------------------


class TestVulnEntry:

    def test_construction(self):
        v = VulnEntry(
            id="CVE-2024-1234", severity="HIGH", package="openssl",
            installed_version="3.0.13", fixed_version="3.0.14",
            title="Buffer overflow", scanner="trivy",
        )
        assert v.id == "CVE-2024-1234"
        assert v.severity == "HIGH"
        assert v.scanner == "trivy"

    def test_defaults(self):
        v = VulnEntry(id="CVE-2024-0001", severity="LOW", package="zlib",
                       installed_version="1.2.13")
        assert v.fixed_version == ""
        assert v.title == ""
        assert v.scanner == ""

    def test_roundtrip(self):
        v = VulnEntry(id="CVE-2024-5678", severity="CRITICAL", package="curl",
                       installed_version="7.88", fixed_version="7.89",
                       title="RCE", scanner="grype")
        d = v.model_dump()
        v2 = VulnEntry.model_validate(d)
        assert v == v2


# ---------------------------------------------------------------------------
# ArchManifest
# ---------------------------------------------------------------------------


class TestArchManifest:

    def test_construction(self):
        m = ArchManifest(
            arch="arm64",
            packages=[PackageEntry(name="bash", version="5.0", source="dpkg")],
            assets=[AssetEntry(filename="vmlinuz", hash="a" * 64, size=100)],
            vulns=[VulnEntry(id="CVE-2024-1", severity="LOW", package="x", installed_version="1")],
        )
        assert m.arch == "arm64"
        assert len(m.packages) == 1
        assert len(m.assets) == 1
        assert len(m.vulns) == 1

    def test_empty_arch_rejected(self):
        with pytest.raises(ValidationError, match="arch must not be empty"):
            ArchManifest(arch="")

    def test_empty_lists_ok(self):
        m = ArchManifest(arch="x86_64")
        assert m.packages == []
        assert m.assets == []
        assert m.vulns == []


# ---------------------------------------------------------------------------
# ImageManifest
# ---------------------------------------------------------------------------


class TestImageManifest:

    def test_construction(self):
        m = ImageManifest(
            version="0.12.1",
            created="2026-03-26T12:00:00Z",
            architectures={"arm64": ArchManifest(arch="arm64")},
        )
        assert m.version == "0.12.1"
        assert "arm64" in m.architectures

    def test_empty_version_rejected(self):
        with pytest.raises(ValidationError, match="version must not be empty"):
            ImageManifest(version="", created="2026-01-01T00:00:00Z")

    def test_json_roundtrip(self):
        m = ImageManifest(
            version="1.0.0", created="2026-01-01T00:00:00Z",
            architectures={"arm64": ArchManifest(arch="arm64")},
        )
        j = m.model_dump_json()
        m2 = ImageManifest.model_validate_json(j)
        assert m == m2

    def test_empty_architectures_ok(self):
        m = ImageManifest(version="1.0.0", created="2026-01-01T00:00:00Z")
        assert m.architectures == {}


# ---------------------------------------------------------------------------
# parse_dpkg_query
# ---------------------------------------------------------------------------


class TestParseDpkgQuery:

    def test_happy_path(self):
        pkgs = parse_dpkg_query(DPKG_OUTPUT)
        assert len(pkgs) == 5
        assert pkgs[0].name == "bash"
        assert pkgs[0].version == "5.2.15-2+b2"
        assert pkgs[0].arch == "arm64"
        assert pkgs[0].source == "dpkg"

    def test_single_line(self):
        pkgs = parse_dpkg_query(DPKG_SINGLE)
        assert len(pkgs) == 1

    def test_empty_output(self):
        assert parse_dpkg_query("") == []
        assert parse_dpkg_query("\n") == []

    def test_malformed_line_skipped(self):
        output = "bash\t5.2\tarm64\nbroken line no tabs\ncurl\t7.88\tarm64\n"
        pkgs = parse_dpkg_query(output)
        assert len(pkgs) == 2

    def test_sorted_by_name(self):
        output = "zsh\t5.9\tarm64\nbash\t5.2\tarm64\n"
        pkgs = parse_dpkg_query(output)
        assert pkgs[0].name == "bash"
        assert pkgs[1].name == "zsh"


# ---------------------------------------------------------------------------
# parse_pip_list
# ---------------------------------------------------------------------------


class TestParsePipList:

    def test_happy_path(self):
        pkgs = parse_pip_list(PIP_JSON)
        assert len(pkgs) == 3
        assert pkgs[1].name == "pytest"  # sorted
        assert pkgs[1].source == "pip"

    def test_empty_array(self):
        assert parse_pip_list(PIP_EMPTY) == []

    def test_invalid_json(self):
        with pytest.raises(ValueError, match="Invalid"):
            parse_pip_list("not json")

    def test_sorted_by_name(self):
        pkgs = parse_pip_list(PIP_JSON)
        names = [p.name for p in pkgs]
        assert names == sorted(names)


# ---------------------------------------------------------------------------
# parse_npm_ls
# ---------------------------------------------------------------------------


class TestParseNpmLs:

    def test_happy_path(self):
        pkgs = parse_npm_ls(NPM_JSON)
        assert len(pkgs) == 2
        assert pkgs[0].source == "npm"

    def test_empty_dependencies(self):
        assert parse_npm_ls(NPM_EMPTY) == []

    def test_no_dependencies_key(self):
        assert parse_npm_ls(NPM_NO_DEPS_KEY) == []

    def test_invalid_json(self):
        with pytest.raises(ValueError, match="Invalid"):
            parse_npm_ls("not json")

    def test_sorted_by_name(self):
        pkgs = parse_npm_ls(NPM_JSON)
        names = [p.name for p in pkgs]
        assert names == sorted(names)


# ---------------------------------------------------------------------------
# parse_b3sums
# ---------------------------------------------------------------------------


class TestParseB3sums:

    def test_happy_path(self):
        assets = parse_b3sums(B3SUMS_OUTPUT)
        assert len(assets) == 3
        assert assets[0].filename == "vmlinuz"
        assert len(assets[0].hash) == 64

    def test_with_sizes(self):
        assets = parse_b3sums(B3SUMS_OUTPUT, sizes=SIZES)
        assert assets[0].size == 12_345_678
        assert assets[2].size == 999_999_999

    def test_without_sizes_defaults_zero(self):
        assets = parse_b3sums(B3SUMS_OUTPUT)
        assert all(a.size == 0 for a in assets)

    def test_empty_output(self):
        assert parse_b3sums("") == []
        assert parse_b3sums("\n") == []


# ---------------------------------------------------------------------------
# collect_bom
# ---------------------------------------------------------------------------


class TestCollectBom:

    def test_all_combined(self):
        m = collect_bom(
            arch="arm64",
            dpkg_output=DPKG_SINGLE,
            pip_output=PIP_EMPTY,
            npm_output=NPM_EMPTY,
            b3sum_output=B3SUMS_OUTPUT,
        )
        assert m.arch == "arm64"
        assert len(m.packages) == 1
        assert len(m.assets) == 3

    def test_empty_outputs(self):
        m = collect_bom(arch="x86_64", dpkg_output="", pip_output="[]",
                        npm_output=NPM_NO_DEPS_KEY, b3sum_output="")
        assert m.packages == []
        assert m.assets == []


# ---------------------------------------------------------------------------
# render
# ---------------------------------------------------------------------------


class TestRender:

    def _make_manifest(self, *, vulns=None):
        pkgs = [
            PackageEntry(name="bash", version="5.2.15", source="dpkg", arch="arm64"),
            PackageEntry(name="pytest", version="8.3.4", source="pip"),
        ]
        assets = [
            AssetEntry(filename="vmlinuz", hash="a" * 64, size=12_345_678),
        ]
        arch = ArchManifest(arch="arm64", packages=pkgs, assets=assets,
                            vulns=vulns or [])
        return ImageManifest(
            version="0.12.1", created="2026-03-26T12:00:00Z",
            architectures={"arm64": arch},
        )

    def test_header(self):
        text = render(self._make_manifest())
        assert "0.12.1" in text
        assert "2026-03-26" in text

    def test_arch_section(self):
        text = render(self._make_manifest())
        assert "arm64" in text

    def test_dpkg_packages(self):
        text = render(self._make_manifest())
        assert "bash" in text
        assert "5.2.15" in text

    def test_pip_packages(self):
        text = render(self._make_manifest())
        assert "pytest" in text
        assert "8.3.4" in text

    def test_assets(self):
        text = render(self._make_manifest())
        assert "vmlinuz" in text
        assert "11.8 MB" in text or "12.3 MB" in text or "MB" in text

    def test_vulns_section(self):
        vulns = [VulnEntry(id="CVE-2024-1234", severity="HIGH", package="openssl",
                           installed_version="3.0.13", fixed_version="3.0.14")]
        text = render(self._make_manifest(vulns=vulns))
        assert "CVE-2024-1234" in text
        assert "HIGH" in text

    def test_no_vulns_omitted(self):
        text = render(self._make_manifest())
        assert "Vulnerabilities" not in text

    def test_multi_arch(self):
        arm = ArchManifest(arch="arm64", packages=[
            PackageEntry(name="bash", version="5.2", source="dpkg"),
        ])
        x86 = ArchManifest(arch="x86_64", packages=[
            PackageEntry(name="bash", version="5.2", source="dpkg"),
        ])
        m = ImageManifest(version="1.0.0", created="2026-01-01T00:00:00Z",
                          architectures={"arm64": arm, "x86_64": x86})
        text = render(m)
        assert "arm64" in text
        assert "x86_64" in text

    def test_empty_manifest(self):
        m = ImageManifest(version="1.0.0", created="2026-01-01T00:00:00Z")
        text = render(m)
        assert "1.0.0" in text
