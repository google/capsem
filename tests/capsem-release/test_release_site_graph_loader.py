"""Release-site graph loader gates."""

from __future__ import annotations

from helpers.release_site import build_release_site_from_fixture


def test_release_site_builds_from_release_graph_fixture() -> None:
    dist = build_release_site_from_fixture()

    index = (dist / "index.html").read_text(encoding="utf-8")
    stable = (dist / "channels" / "stable" / "index.html").read_text(encoding="utf-8")
    package_detail = (
        dist / "channels" / "stable" / "packages" / "capsem-1-4-0-pkg" / "index.html"
    ).read_text(encoding="utf-8")
    profile = (
        dist / "channels" / "stable" / "profiles" / "co-work" / "index.html"
    ).read_text(encoding="utf-8")

    assert "/assets/stable/manifest.json" in index
    assert "/manifests/stable/" not in index
    assert "Stable" in index
    assert "Nightly" in index
    assert "Manifest revision" in index
    assert "1.0.2" in index
    assert "1.5.0-nightly.20260702" not in index
    assert "Capsem-1.4.0.pkg" not in index
    assert "rootfs.erofs" not in index
    assert "Capsem-1.4.0.pkg" in stable
    assert "SPDXRef-File-capsem" not in stable
    assert "SPDXRef-File-capsem" in package_detail
    assert "rootfs.erofs" not in stable
    assert "1.0.0-stable.20260702" in profile
    assert "Minimum Capsem" in profile
    assert "ABOM" in profile
