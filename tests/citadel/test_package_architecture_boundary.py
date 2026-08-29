"""Citadel guard: the two architecture vocabularies must never cross.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. Why this one exists is in `ARCHITECTURE_DOMAIN_RATIONALE` below --
stated there rather than only here, so a violation prints it instead of a bare
comparison against a JSON blob.
"""

from __future__ import annotations

import json
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)

ARCHITECTURE_DOMAIN_RATIONALE = """\
Package architecture and machine architecture are different vocabularies.

    PackageArchitecture   amd64 | arm64     what a .deb or .pkg calls itself
    Architecture          x86_64 | arm64    what a machine and its assets are

They overlap on `arm64` and disagree on the other one, which is exactly what
makes crossing them survivable long enough to ship. The failure modes are a
`capsem_0.6.0_x86_64.deb` that no Debian tool will install, an `amd64` entry in
a profile's architecture list that matches no asset, and a release graph whose
packages and binaries disagree about what they are.

The rule is typed domains, not string conversion. `capsem-core` owns both
enums; inventory rows declare which one they hold; and the helpers that used to
bridge them by sniffing text -- `package_architecture_for_name`,
`name.contains("amd64")`, `deb_graph_arch` -- are forbidden by name here
because each was a place the two vocabularies quietly became one.

This is the same class of defect as sharing strings between BuildKit and
container network modes, which once sent `bridge` to `docker build`. When two
closed vocabularies can be spelled alike, give them types and let the compiler
refuse the crossing.

See AGENTS.md and skills/release-process/SKILL.md.
"""


def test_package_and_machine_architecture_vocabularies_never_cross() -> None:
    manifest = json.loads(FIXTURE.read_text())

    for channel in manifest["manifests"].values():
        for release in channel.values():
            for package in release["packages"]:
                architecture = package["architecture"]
                assert architecture in {"amd64", "arm64"}, (
                    ARCHITECTURE_DOMAIN_RATIONALE + f"\n{package}"
                )
                assert all(
                    binary["architecture"] == architecture for binary in package["binaries"]
                ), ARCHITECTURE_DOMAIN_RATIONALE + f"\n{package}"
                if package["kind"] == "debian_package":
                    assert package["platform"] == "linux"
                    assert package["name"].endswith(f"_{architecture}.deb")
                    assert "_x86_64.deb" not in package["name"]
                else:
                    assert package["kind"] == "macos_pkg"
                    assert package["platform"] == "macos"
                    assert architecture == "arm64"
                    assert package["name"].endswith(".pkg")

            for profile in release["profiles"].values():
                for architecture in profile["architectures"]:
                    assert architecture["architecture"] in {"arm64", "x86_64"}, (
                        ARCHITECTURE_DOMAIN_RATIONALE + f"\n{architecture}"
                    )
                    assert architecture["architecture"] != "amd64", (
                        ARCHITECTURE_DOMAIN_RATIONALE
                        + "\na profile architecture used the package vocabulary: "
                        + f"{architecture}"
                    )


def test_rust_graph_uses_distinct_typed_architecture_domains() -> None:
    core = (PROJECT_ROOT / "crates" / "capsem-core" / "src" / "asset_manager.rs").read_text()
    source = (PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs").read_text()
    main = (PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs").read_text()
    updater = (PROJECT_ROOT / "crates" / "capsem" / "src" / "update.rs").read_text()

    assert "pub enum PackageArchitecture {" in core
    assert "Amd64," in core
    assert "pub use capsem_core::asset_manager::{Architecture, PackageArchitecture};" in source
    package_row = source.split("pub struct PackageInventoryRow", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]
    binary_row = source.split("pub struct BinaryInventoryRow", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]
    software_row = source.split("pub struct SoftwareInventoryRow", maxsplit=1)[1].split(
        "}", maxsplit=1
    )[0]
    violations: list[str] = []
    for row, name in (
        (package_row, "PackageInventoryRow"),
        (binary_row, "BinaryInventoryRow"),
    ):
        if "pub architecture: PackageArchitecture" not in row:
            violations.append(f"{name} no longer declares PackageArchitecture")
    if "pub architecture: Architecture" not in software_row:
        violations.append("SoftwareInventoryRow no longer declares Architecture")
    for text, where in ((updater, "capsem/src/update.rs"),):
        for needed in ("architecture: PackageArchitecture", "architecture: Architecture"):
            if needed not in text:
                violations.append(f"{where} no longer carries `{needed}`")

    # Each of these was a real bridge between the two vocabularies, removed
    # once. They are named rather than pattern-matched so the failure says
    # which crossing came back.
    for text, where, needle, reason in (
        (
            main,
            "capsem-admin/src/main.rs",
            "fn package_architecture_for_name(name: &str) -> String",
            "converts a package name to an architecture by parsing text",
        ),
        (
            main,
            "capsem-admin/src/main.rs",
            'name.contains("amd64")',
            "sniffs the package vocabulary out of a filename",
        ),
        (
            updater,
            "capsem/src/update.rs",
            "fn deb_graph_arch",
            "translates between the two domains instead of holding both types",
        ),
    ):
        if needle in text:
            violations.append(f"{where} contains `{needle}` ({reason})")

    assert not violations, ARCHITECTURE_DOMAIN_RATIONALE + "\n" + "\n".join(violations)


def test_public_linux_package_consumers_use_debian_identity() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    validator = (
        PROJECT_ROOT
        / "build_system/builder/release/tools/check_public_binary_release.py"
    ).read_text()

    # A Debian package is `amd64`. Anything downstream asking it for `x86_64`
    # is asking the machine vocabulary of a package, and will either match
    # nothing or match the wrong artifact.
    violations = [
        f"{where} contains `{needle}` (asks a Debian package for a machine architecture)"
        for text, where, needle in (
            (workflow, ".github/workflows/release.yaml", "linux/x86_64 package"),
            (workflow, ".github/workflows/release.yaml", "package.get('architecture') == 'x86_64'"),
            (
                validator,
                "scripts/check-public-binary-release.py",
                'RequiredPackage("linux", "x86_64", "debian_package")',
            ),
            (
                validator,
                "scripts/check-public-binary-release.py",
                'package.get("architecture") != "x86_64"',
            ),
            (
                validator,
                "scripts/check-public-binary-release.py",
                'package.get("architecture") == "x86_64"',
            ),
        )
        if needle in text
    ]

    assert not violations, ARCHITECTURE_DOMAIN_RATIONALE + "\n" + "\n".join(violations)
