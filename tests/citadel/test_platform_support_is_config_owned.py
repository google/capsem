"""Fast guards for the one user-facing claim about what Capsem runs on.

These floors were written out longhand in both public `install.sh` copies, in
the Tauri bundle, in the README and in the docs, with nothing comparing them --
and they had already drifted: the installers refused anything below macOS 14
while the app bundle advertised 13.0, so the bundle promised a release the
installer denied.

`config/gate.toml` owns the claim. `install.sh` is served to users as a shell
script and cannot read it at runtime, so the value is copied there and compared
here rather than injected. What the Linux floor *means* is proved separately
and against real images by `build_system/packaging/linux/prove-deb-platform-support.py`; this only
keeps the surfaces that state it from disagreeing.
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
GATE_CONFIG = PROJECT_ROOT / "config/gate.toml"
CACHE_CONFIG = PROJECT_ROOT / "config/cache.toml"
README = PROJECT_ROOT / "README.md"
DOCS = PROJECT_ROOT / "web" / "docs" / "src" / "content" / "docs" / "getting-started.md"
TAURI = PROJECT_ROOT / "crates/capsem-app/tauri.conf.json"
INSTALLERS = (
    PROJECT_ROOT / "web/marketing/public/install.sh",
    PROJECT_ROOT / "web" / "docs" / "public" / "install.sh",
)


def _platforms() -> dict:
    return tomllib.loads(GATE_CONFIG.read_text(encoding="utf-8"))["platforms"]


def _supported() -> list[dict]:
    """The releases whose libc satisfies the declared floor."""
    linux = _platforms()["linux"]
    floor = tuple(int(part) for part in linux["minimum_glibc"].split("."))
    supported = []
    for row in linux["distributions"]:
        flavour, _, value = row["libc"].partition(" ")
        if flavour == "glibc" and tuple(int(p) for p in value.split(".")) >= floor:
            supported.append(row)
    return supported


def _oldest_per_name() -> dict[str, str]:
    oldest: dict[str, str] = {}
    for row in _supported():
        key = tuple(int(part) for part in row["version"].split("."))
        current = oldest.get(row["name"])
        if current is None or key < tuple(int(p) for p in current.split(".")):
            oldest[row["name"]] = row["version"]
    return oldest


def test_every_probe_records_the_libc_the_claim_is_derived_from() -> None:
    """Without it the support claim cannot be answered without Docker."""
    for row in _platforms()["linux"]["distributions"]:
        flavour, _, value = row["libc"].partition(" ")
        assert flavour in {"glibc", "musl"}, row
        assert value and all(part.isdigit() for part in value.split(".")), row


def test_probe_images_are_pinned_by_digest() -> None:
    """A moving tag would silently change what was proved."""
    for row in _platforms()["linux"]["distributions"]:
        assert row["digest"].startswith("sha256:"), row
        assert len(row["digest"]) == len("sha256:") + 64, row


def test_cache_controller_probe_image_is_pinned_by_digest() -> None:
    """Capacity enforcement must execute the same image on every run."""
    image = tomllib.loads(CACHE_CONFIG.read_text(encoding="utf-8"))["control"]["docker"][
        "capacity_probe_image"
    ]
    _, separator, digest = image.rpartition("@sha256:")

    assert separator and len(digest) == 64 and all(char in "0123456789abcdef" for char in digest)


def test_readme_badges_match_the_supported_releases() -> None:
    readme = README.read_text(encoding="utf-8")
    macos = _platforms()["macos"]["minimum_version"].split(".")[0]

    assert f"badge/macOS-{macos}%2B" in readme
    for name, version in _oldest_per_name().items():
        assert f"badge/{name}-{version}%2B" in readme, f"no README badge for {name} {version}"


def _support_row(text: str, system: str) -> str:
    """The one table row naming ``system``, so a claim is read per system."""
    rows = [
        line
        for line in text.splitlines()
        if line.startswith("|") and line.split("|")[1].strip() == system
    ]
    assert len(rows) == 1, f"expected exactly one support row for {system}, found {len(rows)}"
    return rows[0]


def test_readme_and_docs_state_the_same_supported_releases() -> None:
    macos = _platforms()["macos"]
    major = macos["minimum_version"].split(".")[0]

    for surface in (README, DOCS):
        text = surface.read_text(encoding="utf-8")
        row = _support_row(text, "macOS")
        assert f"{major} ({macos['minimum_release_name']}) or later" in row, surface.name
        for name, version in _oldest_per_name().items():
            row = _support_row(text, name)
            assert f"{version} or later" in row, (
                f"{surface.name} row for {name} does not state {version}"
            )


def test_the_app_bundle_does_not_advertise_a_release_the_installer_refuses() -> None:
    """The exact drift this guard was written for."""
    declared = json.loads(TAURI.read_text(encoding="utf-8"))["bundle"]["macOS"][
        "minimumSystemVersion"
    ]

    assert declared == _platforms()["macos"]["minimum_version"]


def test_both_public_installers_enforce_the_configured_macos_floor() -> None:
    major = _platforms()["macos"]["minimum_version"].split(".")[0]
    name = _platforms()["macos"]["minimum_release_name"]

    for installer in INSTALLERS:
        text = installer.read_text(encoding="utf-8")
        assert f'"$MACOS_MAJOR" -lt {major}' in text, installer.name
        assert f"requires macOS {major} ({name}) or later" in text, installer.name


def test_the_two_public_installers_are_the_same_file() -> None:
    """They are hand-maintained copies, so only equality keeps them honest."""
    site, docs = (path.read_text(encoding="utf-8") for path in INSTALLERS)

    assert site == docs, "web/marketing/public/install.sh and web/docs/public/install.sh have diverged"
