"""Fast guards for the publishable guest-rootfs composition budget."""

from __future__ import annotations

import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
BUILD_CONFIG = PROJECT_ROOT / "config/docker/image/build.toml"
BUILDER = PROJECT_ROOT / "src/capsem/builder/docker.py"
DEPENDENCY_TEMPLATE = PROJECT_ROOT / "config/docker/Dockerfile.rootfs-dependencies.j2"
PROFILE_BUILD_SCRIPTS = (
    PROJECT_ROOT / "config/profiles/code/build.sh",
    PROJECT_ROOT / "config/profiles/co-work/build.sh",
)

OLLAMA_ROOTS = ("usr/lib/ollama", "usr/local/lib/ollama")
ACCELERATOR_FAMILIES = (
    "cuda",
    "hip",
    "jetpack",
    "oneapi",
    "opencl",
    "rocm",
    "vulkan",
)

ROOTFS_BUDGET_RATIONALE = """\
The stable profile release grew each rootfs from below 1 GB to almost 3 GB when
Ollama moved bundled CUDA/Vulkan libraries from /usr/local/lib/ollama to
/usr/lib/ollama. GitHub then rejected the exact release assets after more than
two hours of qualification. The guest has no accelerator device, so these
libraries are dead payload. Keep both the composition exclusion and the raw
and packed size checks in the ordinary build rail. See skills/build-images.
"""


def _rootfs_config() -> dict:
    return tomllib.loads(BUILD_CONFIG.read_text())["build"]["rootfs"]


def _erofs_config() -> dict:
    return tomllib.loads(BUILD_CONFIG.read_text())["build"]["erofs"]


def test_publishable_rootfs_has_independent_raw_and_950_mb_packed_ceilings() -> None:
    rootfs = _rootfs_config()

    assert rootfs["max_erofs_bytes"] == 950_000_000, ROOTFS_BUDGET_RATIONALE
    assert rootfs["max_uncompressed_bytes"] > rootfs["max_erofs_bytes"], ROOTFS_BUDGET_RATIONALE


def test_every_known_ollama_accelerator_family_is_forbidden_at_both_roots() -> None:
    configured = set(_rootfs_config()["forbidden_path_prefixes"])
    expected = {f"{root}/{family}" for root in OLLAMA_ROOTS for family in ACCELERATOR_FAMILIES}

    assert expected <= configured, ROOTFS_BUDGET_RATIONALE


def test_profile_hooks_remove_and_prove_the_same_accelerator_families() -> None:
    failures: list[str] = []
    for path in PROFILE_BUILD_SCRIPTS:
        source = path.read_text()
        for root in ("/usr/lib/ollama", "/usr/local/lib/ollama"):
            if root not in source:
                failures.append(f"{path}: does not inspect {root}")
        for family in ACCELERATOR_FAMILIES:
            if f'"$ollama_root"/{family}*' not in source:
                failures.append(f"{path}: does not remove {family}")
            if f"-name '{family}*'" not in source:
                failures.append(f"{path}: does not prove {family} is absent")
        if "forbidden Ollama accelerator payload survived cleanup" not in source:
            failures.append(f"{path}: cleanup is not fail-closed")

    assert not failures, ROOTFS_BUDGET_RATIONALE + "\n" + "\n".join(failures)


def test_ordinary_rootfs_build_checks_both_sides_of_compression() -> None:
    source = BUILDER.read_text()
    export = source.index("validate_rootfs_export(tar_path, config.build.rootfs)")
    pack = source.index("create_erofs(", export)
    packed_check = source.index("validate_erofs_size(erofs_path, config.build.rootfs)", pack)
    ledger = source.index("_file_ledger_entry(erofs_path", packed_check)

    assert export < pack < packed_check < ledger, ROOTFS_BUDGET_RATIONALE


def test_release_erofs_uses_a_bounded_large_physical_cluster() -> None:
    cluster_size = _erofs_config().get("cluster_size")

    assert cluster_size is not None, ROOTFS_BUDGET_RATIONALE
    assert cluster_size >= 65536, ROOTFS_BUDGET_RATIONALE
    assert cluster_size <= 1048576, ROOTFS_BUDGET_RATIONALE
    assert cluster_size & (cluster_size - 1) == 0, ROOTFS_BUDGET_RATIONALE


def test_dependency_materializer_drops_package_manager_and_temp_residue() -> None:
    source = DEPENDENCY_TEMPLATE.read_text()

    for path in ("/var/lib/apt/lists/*", "/var/cache/apt/*", "/tmp/*"):
        assert path in source, ROOTFS_BUDGET_RATIONALE
