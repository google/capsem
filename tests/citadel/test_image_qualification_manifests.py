"""Every public image declares how it qualifies, in terms the harness knows.

Shipping an image means booting it under Capsem first (tests/qualification).
The mandatory groups need no declaration; `images/<name>/qualify.toml` adds
the groups its capabilities call for. A catalog image without a manifest, or
one naming a capability the harness has no group for, would publish untested
behavior -- so neither gets past the fast phase.
"""

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
#: The capability groups tests/qualification implements.
KNOWN = {"agent", "surface", "ollama", "toolchain"}


def public_images() -> set[str]:
    return set(tomllib.loads((ROOT / "images/catalog.toml").read_text())["images"])


def test_every_catalog_image_has_a_qualification_manifest():
    missing = sorted(name for name in public_images() if not (ROOT / "images" / name / "qualify.toml").is_file())
    assert not missing, f"catalog images with no images/<name>/qualify.toml: {missing}"


def test_a_manifest_declares_only_capabilities_the_harness_tests():
    for name in sorted(public_images()):
        declared = tomllib.loads((ROOT / "images" / name / "qualify.toml").read_text())
        assert set(declared) <= {"capabilities", "expect"}, (name, sorted(declared))
        unknown = set(declared.get("capabilities", ())) - KNOWN
        assert not unknown, f"{name} declares capabilities with no qualification group: {sorted(unknown)}"


def test_internal_images_are_not_qualified_as_products():
    """base and capsem-debug never reach the catalog, so they carry no manifest."""
    for internal in ("base", "capsem-debug"):
        assert internal not in public_images()
        assert not (ROOT / "images" / internal / "qualify.toml").exists(), internal
