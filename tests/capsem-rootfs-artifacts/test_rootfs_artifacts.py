"""Verify rootfs artifacts are consistent across build context, Dockerfile, and doctor checks."""

import importlib
import tempfile

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ARTIFACTS_DIR = PROJECT_ROOT / "guest" / "artifacts"
CONFIG_DIR = PROJECT_ROOT / "config"

pytestmark = pytest.mark.rootfs

from capsem.builder.docker import (
    GUEST_BINARIES,
    ROOTFS_SCRIPTS,
    ROOTFS_SCRIPT_DIRS,
    ROOTFS_SUPPORT_FILES,
)

# The canonical list of required rootfs artifacts (files and dirs)
REQUIRED_FILES = ["capsem-init", *ROOTFS_SUPPORT_FILES, *ROOTFS_SCRIPTS]
REQUIRED_DIRS = list(ROOTFS_SCRIPT_DIRS)


class TestArtifactsExist:

    @pytest.mark.parametrize("name", REQUIRED_FILES)
    def test_required_file_exists(self, name):
        """Each required artifact file exists in guest/artifacts/."""
        path = ARTIFACTS_DIR / name
        assert path.is_file(), f"Missing artifact file: {path}"

    @pytest.mark.parametrize("name", REQUIRED_DIRS)
    def test_required_dir_exists(self, name):
        """Each required artifact directory exists in guest/artifacts/."""
        path = ARTIFACTS_DIR / name
        assert path.is_dir(), f"Missing artifact directory: {path}"

    def test_ca_cert_exists(self):
        """CA certificate exists in config/."""
        ca = CONFIG_DIR / "capsem-ca.crt"
        assert ca.is_file(), f"Missing CA certificate: {ca}"


class TestBuildContext:

    def test_prepare_build_context_copies_all(self):
        """prepare_build_context copies all required artifacts to context dir."""
        try:
            from capsem.builder.docker import prepare_build_context
            from capsem.builder.config import load_guest_config
        except ImportError:
            pytest.skip("capsem-builder not installed")

        guest_dir = PROJECT_ROOT / "guest"
        if not (guest_dir / "config").exists():
            pytest.skip("No guest config directory found")

        config = load_guest_config(guest_dir)
        arch_name = list(config.build.architectures.keys())[0]

        with tempfile.TemporaryDirectory() as tmp:
            context_dir = Path(tmp)
            prepare_build_context(
                config, arch_name, "Dockerfile.rootfs.j2",
                context_dir, PROJECT_ROOT,
            )
            # Verify all required files were copied
            assert (context_dir / "capsem-ca.crt").exists(), "CA cert not in context"
            for name in REQUIRED_FILES:
                if name == "capsem-init":
                    continue  # capsem-init goes into kernel context, not rootfs
                assert (context_dir / name).exists(), f"{name} not in context"
            for name in REQUIRED_DIRS:
                assert (context_dir / name).exists(), f"{name}/ not in context"


class TestRootfsValidationContract:

    def test_guest_binaries_include_release_critical_helpers(self):
        """The canonical guest list includes helpers required by capsem-init."""
        assert "capsem-dns-proxy" in GUEST_BINARIES
        assert "capsem-sysutil" in GUEST_BINARIES

    def test_validate_rootfs_derives_binary_requirements_from_guest_binaries(self):
        """The release validator imports GUEST_BINARIES and checks /usr/local/bin."""
        validator = (PROJECT_ROOT / "scripts" / "validate-rootfs.sh").read_text()
        assert "GUEST_BINARIES" in validator
        assert "for name in [*GUEST_BINARIES, *ROOTFS_SCRIPTS]" in validator
        assert 'print(f"file /usr/local/bin/{name}")' in validator


class TestDoctorConsistency:

    def test_doctor_check_source_files_passes(self):
        """doctor check_source_files passes on this repo."""
        try:
            from capsem.builder.doctor import check_source_files
        except ImportError:
            pytest.skip("capsem-builder not installed")

        result = check_source_files(PROJECT_ROOT)
        assert result.passed, f"Doctor source file check failed: {result.detail}"

    def test_no_hardcoded_artifact_lists(self):
        """Key modules import artifact lists rather than hardcoding them.

        This is aspirational -- checks that prepare_build_context, doctor, and
        validate all reference the same artifacts.
        """
        # Read the three source files and check they reference the same artifacts
        docker_src = (PROJECT_ROOT / "src/capsem/builder/docker.py").read_text()
        doctor_src = (PROJECT_ROOT / "src/capsem/builder/doctor.py").read_text()

        for constant in ("ROOTFS_SCRIPTS", "ROOTFS_SCRIPT_DIRS", "ROOTFS_SUPPORT_FILES"):
            assert constant in docker_src, f"docker.py missing {constant}"
            assert constant in doctor_src, f"doctor.py missing {constant}"
