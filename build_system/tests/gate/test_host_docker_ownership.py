"""Host build Docker resources have one build-system owner."""

from __future__ import annotations

import tomllib
from pathlib import Path

from helpers.source_modes import tracked_source_modes

ROOT = Path(__file__).resolve().parents[3]
OWNER = ROOT / "build_system/docker"
EXPECTED = {
    ".dockerignore",
    "Dockerfile.asset-tools",
    "Dockerfile.guest-rust-builder",
    "Dockerfile.host-builder",
    "Dockerfile.install-builder",
    "Dockerfile.install-test",
    "Dockerfile.linux-rust",
    "Dockerfile.linux-rust-base",
    "Dockerfile.package",
    "Dockerfile.package-builder",
    "materialize-install-os.sh",
    "sources-multiarch.sh",
    "swap-dev-libs.sh",
}


def _toml(path: Path) -> dict[str, object]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def test_host_docker_resources_have_one_exact_owner() -> None:
    found = {
        path.relative_to(OWNER).as_posix()
        for path in OWNER.rglob("*")
        if path.is_file()
    }
    assert found == EXPECTED
    assert not (ROOT / "docker").exists()
    assert tracked_source_modes(ROOT, OWNER) == dict.fromkeys(EXPECTED, 0o644)


def test_product_image_templates_and_root_secret_exclusions_stay_owned() -> None:
    product = ROOT / "config/docker"
    assert (product / "Dockerfile.kernel.j2").is_file()
    assert (product / "Dockerfile.rootfs.j2").is_file()
    assert (product / "image/build.toml").is_file()
    assert "**/private" in (ROOT / ".dockerignore").read_text(encoding="utf-8")
    assert (OWNER / ".dockerignore").read_text(encoding="utf-8") == "README*\n"


def test_gate_and_guest_build_config_resolve_the_new_owner() -> None:
    gate = _toml(ROOT / "config/gate.toml")
    host = gate["hostimage"]
    assert isinstance(host, dict)
    assert host["context"] == "build_system/docker/"
    assert host["dockerfile"] == "build_system/docker/Dockerfile.host-builder"
    assert host["base_dockerfile"] == "build_system/docker/Dockerfile.linux-rust-base"
    assert host["lane_dockerfile"] == "build_system/docker/Dockerfile.linux-rust"

    install = gate["install"]
    package = gate["package"]
    assert isinstance(install, dict) and isinstance(package, dict)
    assert install["dockerfile"] == "build_system/docker/Dockerfile.install-test"
    assert install["builder"]["dockerfile"] == "build_system/docker/Dockerfile.install-builder"
    assert package["lane_dockerfile"] == "build_system/docker/Dockerfile.package"
    assert package["builder"]["dockerfile"] == "build_system/docker/Dockerfile.package-builder"

    image = _toml(ROOT / "config/docker/image/build.toml")["build"]
    assert isinstance(image, dict)
    assert image["guest_rust_builder"]["dockerfile"] == (
        "build_system/docker/Dockerfile.guest-rust-builder"
    )
    assert image["asset_tools"]["dockerfile"] == (
        "build_system/docker/Dockerfile.asset-tools"
    )


def test_repository_context_copies_name_the_moved_helpers() -> None:
    install = (OWNER / "Dockerfile.install-builder").read_text(encoding="utf-8")
    package = (OWNER / "Dockerfile.package-builder").read_text(encoding="utf-8")
    host = (OWNER / "Dockerfile.host-builder").read_text(encoding="utf-8")
    assert "COPY --chmod=555 build_system/docker/materialize-install-os.sh" in install
    assert "COPY --chmod=555 build_system/docker/swap-dev-libs.sh" in package
    assert (
        "COPY build_system/builder/image/tools/build/materialize_package_ort.py "
        "/usr/local/bin/materialize-package-ort.py"
    ) in package
    assert "COPY sources-multiarch.sh /tmp/" in host
    assert "COPY swap-dev-libs.sh /usr/local/bin/swap-dev-libs" in host


def test_network_open_apt_layers_persist_partial_snapshot_downloads() -> None:
    """A failed immutable snapshot request must not discard completed bytes."""
    config = (ROOT / "config/gate.toml").read_text(encoding="utf-8")
    for name, namespace in (
        ("Dockerfile.install-builder", "install"),
        ("Dockerfile.package-builder", "package"),
    ):
        source = (OWNER / name).read_text(encoding="utf-8")
        assert f'apt_lists_cache_id = "capsem-{namespace}-apt-lists"' in config
        assert f'apt_archives_cache_id = "capsem-{namespace}-apt-archives"' in config
        assert "id=${APT_LISTS_CACHE_ID},target=/var/lib/apt/lists,sharing=locked" in source
        assert "id=${APT_ARCHIVES_CACHE_ID},target=/var/cache/apt,sharing=locked" in source
        assert "rm -f /etc/apt/apt.conf.d/docker-clean" in source

    for helper in ("materialize-install-os.sh", "swap-dev-libs.sh"):
        source = (OWNER / helper).read_text(encoding="utf-8")
        assert "rm -rf /var/lib/apt/lists" not in source
