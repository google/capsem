"""Fixture helpers for hermetic `compose.yaml` `build:` / `Dockerfile` live acceptance."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import platform
import shutil
import socket
import subprocess
import tarfile
from pathlib import Path
from typing import Any


def load_sibling_module(filename: str) -> Any:
    """Load a sibling test module by filename without mutating `sys.modules['tests']`."""
    path = Path(__file__).resolve().with_name(filename)
    spec = importlib.util.spec_from_file_location(f"_capsem_sibling_{path.stem}", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


try:
    import tests.oci_workload_fixture as _oci_fixture
except ImportError:
    _oci_fixture = load_sibling_module("oci_workload_fixture.py")

hermetic_oci_workload_image = _oci_fixture.hermetic_oci_workload_image


def require_docker() -> None:
    """Ensure the `docker` CLI and daemon are available, or fail loudly."""
    if shutil.which("docker") is None:
        raise RuntimeError("docker CLI is required on PATH for host-build live acceptance")
    proc = subprocess.run(["docker", "info"], capture_output=True, text=True, timeout=15)
    if proc.returncode != 0:
        raise RuntimeError(f"docker daemon required for host-build acceptance: {proc.stderr}")


def load_base_image_into_docker(base_tag: str) -> None:
    """Build a minimal busybox OCI tar from the guest initrd and load it into local Docker."""
    busybox = _oci_fixture._extract_busybox_from_initrd(_oci_fixture._find_initrd_path())
    raw_tar = _oci_fixture._build_rootfs_tar_bytes(busybox)
    oci_arch = "arm64" if platform.machine().lower() in ("aarch64", "arm64") else "amd64"
    env_path = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    inner_cfg = {
        "Env": [env_path],
        "Cmd": ["/bin/sh", "-c", "sleep 3600"],
        "WorkingDir": "/",
    }
    rootfs = {
        "type": "layers",
        "diff_ids": ["sha256:" + hashlib.sha256(raw_tar).hexdigest()],
    }
    config_doc = {
        "architecture": oci_arch,
        "os": "linux",
        "config": inner_cfg,
        "rootfs": rootfs,
    }
    config_bytes = json.dumps(config_doc, sort_keys=True).encode("utf-8")
    manifest_doc = [{"Config": "config.json", "RepoTags": [base_tag], "Layers": ["layer.tar"]}]
    manifest_bytes = json.dumps(manifest_doc).encode("utf-8")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tf:
        for name, data in (
            ("config.json", config_bytes),
            ("layer.tar", raw_tar),
            ("manifest.json", manifest_bytes),
        ):
            info = tarfile.TarInfo(name=name)
            info.size = len(data)
            info.mode = 0o644
            tf.addfile(info, io.BytesIO(data))
    subprocess.run(["docker", "load"], input=buf.getvalue(), check=True, capture_output=True)


def write_dockerfile_and_context(root: Path, base_tag: str) -> tuple[Path, Path, Path]:
    """Materialize a multi-stage Dockerfile, compose.yaml, tar archive, and symlink escape."""
    outside = root / "outside"
    outside.mkdir(parents=True, exist_ok=True)
    outside_secret = outside / "secret.txt"
    outside_secret.write_text("host-secret\n", encoding="utf-8")

    project = root / "project"
    sub_dir = project / "sub"
    sub_dir.mkdir(parents=True, exist_ok=True)

    active_escape = project / "active_escape"
    active_escape.symlink_to(outside_secret)

    (project / ".dockerignore").write_text("ignored.txt\nignored_symlink\n", encoding="utf-8")
    (project / "ignored.txt").write_text("should-not-enter-context\n", encoding="utf-8")
    (project / "ignored_symlink").symlink_to(outside_secret)
    (sub_dir / "kept.txt").write_text("kept-ok\n", encoding="utf-8")

    tar_buf = io.BytesIO()
    with tarfile.open(fileobj=tar_buf, mode="w:gz") as tf:
        payload = b"tar-extracted\n"
        info = tarfile.TarInfo(name="unpacked/from_tar.txt")
        info.size = len(payload)
        info.mode = 0o644
        tf.addfile(info, io.BytesIO(payload))
    (sub_dir / "archive.tar.gz").write_bytes(tar_buf.getvalue())

    compose_file = project / "compose.yaml"
    compose_file.write_text(
        "services:\n  default:\n    build:\n      context: .\n      dockerfile: Dockerfile\n"
        "      target: final\n      args:\n        GREETING: from-compose-arg\n",
        encoding="utf-8",
    )
    (project / "Dockerfile").write_text(
        f"FROM {base_tag} AS builder\nARG GREETING=default-greeting\n"
        'RUN echo "stage1:${GREETING}" > /stage1.txt\n'
        'RUN ["/bin/sh", "-c", "echo exec-form-ok >> /stage1.txt"]\n'
        "RUN <<EOF\necho heredoc-run-ok >> /stage1.txt\nEOF\n"
        "COPY <<EOF /inline.txt\nheredoc-copy-ok\nEOF\n"
        "FROM scratch AS assets\nCOPY --from=builder /stage1.txt /stage1.txt\n"
        "COPY --from=builder /inline.txt /inline.txt\n"
        f"FROM {base_tag} AS final\nWORKDIR /opt/app/sub\nENV APP_MODE=production\n"
        "COPY --from=assets /stage1.txt /opt/app/sub/stage1.txt\n"
        "COPY --from=assets --chmod=0640 --chown=1000:1000 /inline.txt /opt/app/sub/app.txt\n"
        "COPY sub/kept.txt /opt/app/sub/kept.txt\nADD sub/archive.tar.gz /opt/app/sub/\n"
        "RUN rm -rf /var/tmp/sandbox-services && "
        "chown 1000:1000 /opt/app/sub && "
        "test -s /usr/local/share/ca-certificates/capsem-ca.crt && "
        'grep -q "BEGIN CERTIFICATE" /usr/local/share/capsem/ca-bundle.crt && '
        "echo ca-trust-ok > /opt/app/sub/ca_verified.txt\n"
        'USER 1000:1000\nENTRYPOINT ["/bin/sh", "-c"]\nCMD ["sleep 3600"]\n',
        encoding="utf-8",
    )
    return compose_file, project, active_escape


def pick_registry_port() -> int:
    """Select loopback port 5055 when free, or an ephemeral loopback port if 5055 is busy."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            sock.bind(("127.0.0.1", 5055))
        except OSError:
            sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def restore_settings(settings_path: Path, prev: str | None) -> None:
    """Restore or remove `settings.toml` after an acceptance test."""
    if prev is None:
        settings_path.unlink(missing_ok=True)
    else:
        settings_path.write_text(prev, encoding="utf-8")


def grant_in_settings(home_dir: Path, reference: str) -> None:
    """Delegate `[images]` admission update in `settings.toml` to the OCI fixture helper."""
    _oci_fixture._grant_in_settings(home_dir, reference)
