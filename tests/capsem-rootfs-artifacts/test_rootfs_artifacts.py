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

    def test_capsem_init_creates_console_fallback_before_redirect(self):
        """PID 1 must keep logging even when devtmpfs omits /dev/console."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()
        redirect = "exec 0<\"$CONSOLE_DEV\" 1>\"$CONSOLE_DEV\" 2>\"$CONSOLE_DEV\""
        fallback = "mknod -m 600 /dev/console c 5 1"
        ttyS0_node = "mknod -m 600 /dev/ttyS0 c 4 64"
        ttyS0 = "CONSOLE_DEV=/dev/ttyS0"
        hvc0 = "CONSOLE_DEV=/dev/hvc0"

        assert fallback in init_script
        assert ttyS0_node in init_script
        assert ttyS0 in init_script
        assert hvc0 in init_script
        assert init_script.index(fallback) < init_script.index(redirect)

    def test_capsem_init_writes_host_visible_stage_markers(self):
        """VirtioFS boots must leave a host-readable stage marker for triage."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()
        stage_file = "/mnt/shared/system/capsem-init-stage.log"
        agent_log = "/mnt/shared/system/capsem-agent.log"

        assert stage_file in init_script
        assert agent_log in init_script
        assert 'init_stage "virtiofs-mounted"' in init_script
        assert 'init_stage "overlay-mounted"' in init_script
        assert 'init_stage "starting-agent"' in init_script
        assert 'init_stage "agent-exited-$AGENT_STATUS"' in init_script

    def test_capsem_init_marks_virtio_block_devices_non_rotational(self):
        """Virtio block disks must advertise non-rotational behavior to Linux."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()

        assert 'echo none > "$dev/queue/scheduler"' in init_script
        assert 'echo 0 > "$dev/queue/rotational"' in init_script
        assert 'echo 4096 > "$dev/queue/read_ahead_kb"' in init_script
        assert 'echo 256 > "$dev/queue/nr_requests"' in init_script

    def test_capsem_init_keeps_network_proxies_alive_or_fails(self):
        """Network proxy launch must survive shell transitions and fail closed."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()

        assert "nohup '$NET_PROXY_PATH' </dev/null >/run/capsem-net-proxy.log 2>&1 &" in init_script
        assert "nohup '$DNS_PROXY_PATH' </dev/null >/run/capsem-dns-proxy.log 2>&1 &" in init_script
        assert "[capsem-init] FATAL: capsem-net-proxy did not become ready" in init_script
        assert "[capsem-init] FATAL: capsem-dns-proxy did not become ready" in init_script
        assert "[capsem-init] FATAL: capsem-net-proxy not found" in init_script
        assert "[capsem-init] FATAL: capsem-dns-proxy not found" in init_script

    def test_capsem_init_keeps_python_venv_off_virtiofs_workspace(self):
        """The Python venv must live on the guest overlay, not /root VirtioFS."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()

        assert "/var/lib/capsem/venv" in init_script
        assert "/root/.venv" not in init_script

    def test_capsem_init_keeps_uv_cache_off_virtiofs_workspace(self):
        """uv cache must live on guest overlay so wheel symlinks work."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()

        assert "UV_CACHE_DIR=/var/cache/capsem/uv" in init_script
        assert "mkdir -p /var/lib/capsem /var/cache/capsem/uv" in init_script

    def test_capsem_init_trusts_guest_git_workspaces(self):
        """Git must work in /root even though VirtioFS files use host uid/gid."""
        init_script = (ARTIFACTS_DIR / "capsem-init").read_text()

        assert "cat > /newroot/etc/gitconfig" in init_script
        assert "directory = *" in init_script
        assert "defaultBranch = main" in init_script

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
