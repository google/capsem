"""Host-build CA grant resolution and context staging."""

from __future__ import annotations

import hashlib
import os
import shutil
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from .build_context import _is_within
from .compose_fields import _realpath
from .dockerfile_ca import patch_dockerfile_for_capsem_ca

CAPSEM_INSPECT_BUILD_CA_PEM_FILE_VAR = "CAPSEM_INSPECT_BUILD_CA_PEM_FILE"
CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE_VAR = "CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE"
_BUILDKIT_FALSE_VALUES = frozenset({"0", "false", "no", "off"})


def _resolve_docker_config_dir(operator_roots: tuple[Path, ...]) -> str | None:
    env_dcfg = os.environ.get("CAPSEM_INSPECT_BUILD_DOCKER_CONFIG", "").strip()
    if not env_dcfg:
        return None
    env_real = _realpath(env_dcfg)
    if not env_real.is_dir():
        raise ValueError(f"CAPSEM_INSPECT_BUILD_DOCKER_CONFIG directory not found: {env_dcfg}")
    if not any(_is_within(env_real, r) for r in operator_roots):
        raise ValueError(
            f"CAPSEM_INSPECT_BUILD_DOCKER_CONFIG {str(env_real)!r} is not within "
            "CAPSEM_INSPECT_ALLOWED_HOST_PATHS"
        )
    return str(env_real)


def _validate_ca_file(raw_path: str, var_name: str, operator_roots: tuple[Path, ...]) -> Path:
    real = Path(os.path.realpath(Path(raw_path).expanduser()))
    if not real.is_file():
        raise ValueError(f"{var_name} file not found: {raw_path}")
    if not any(real == r or real.is_relative_to(r) for r in operator_roots):
        raise ValueError(
            f"{var_name} {str(real)!r} is not within CAPSEM_INSPECT_ALLOWED_HOST_PATHS"
        )
    if "-----BEGIN CERTIFICATE-----" not in real.read_text(encoding="utf-8", errors="replace"):
        raise ValueError(f"{var_name} {str(real)!r} does not contain a PEM certificate")
    return real


def resolve_build_ca_files(
    grant: Any, network: str, operator_roots: tuple[Path, ...]
) -> tuple[str | None, str | None]:
    """Validate `CAPSEM_INSPECT_BUILD_CA_PEM_FILE` and `..._BUNDLE_FILE` (or opt out via `ca_pem=False`)."""
    env_ca = os.environ.get("CAPSEM_INSPECT_BUILD_CA_PEM_FILE", "").strip()
    env_bundle = os.environ.get("CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE", "").strip()
    if not env_ca:
        return None, None
    ca_real = _validate_ca_file(env_ca, CAPSEM_INSPECT_BUILD_CA_PEM_FILE_VAR, operator_roots)
    if grant is not None and getattr(grant, "ca_pem", True) is False:
        return None, None
    if env_bundle:
        b_real = _validate_ca_file(
            env_bundle, CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE_VAR, operator_roots
        )
        return str(ca_real), str(b_real)
    if network != "none":
        raise ValueError(
            f"CA patching with network={network!r} requires "
            f"{CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE_VAR} (full root + Capsem CA bundle)"
        )
    return str(ca_real), str(ca_real)


def compute_ca_fingerprint(spec: Mapping[str, Any]) -> str:
    """Compute SHA-256 fingerprint over the granted CA PEM and full bundle files."""
    if not (ca_file := spec.get("ca_pem_file")):
        return ""
    ca_bytes = Path(str(ca_file)).read_bytes()
    bundle_bytes = Path(str(spec.get("ca_bundle_file") or ca_file)).read_bytes()
    return hashlib.sha256(ca_bytes + b"\0" + bundle_bytes).hexdigest()


def stage_capsem_ca_in_context(src_ctx: Path, dst_ctx: Path, spec: Mapping[str, Any]) -> None:
    """Copy `src_ctx` to `dst_ctx` and stage `.capsem-ca.crt` + `.capsem-ca-bundle.crt`."""
    shutil.copytree(src_ctx, dst_ctx, symlinks=True)
    ca_text = Path(str(spec["ca_pem_file"])).read_text(encoding="utf-8")
    bundle_text = Path(str(spec.get("ca_bundle_file") or spec["ca_pem_file"])).read_text(
        encoding="utf-8"
    )
    if ca_text.strip() and ca_text.strip() not in bundle_text:
        bundle_text = f"{bundle_text.rstrip()}\n{ca_text.strip()}\n"
    (dst_ctx / ".capsem-ca.crt").write_text(ca_text, encoding="utf-8")
    (dst_ctx / ".capsem-ca-bundle.crt").write_text(bundle_text, encoding="utf-8")
    if (di := dst_ctx / ".dockerignore").is_file():
        existing = di.read_text(encoding="utf-8", errors="replace")
        di.write_text(f"{existing}\n!.capsem-ca.crt\n!.capsem-ca-bundle.crt\n", encoding="utf-8")


def prepare_build_inputs(
    spec: Mapping[str, Any],
    tmp_root: Path,
) -> tuple[Path, Path]:
    """Verify BuildKit is not disabled and stage CA files when `ca_pem_file` is configured."""
    if (os.getenv("DOCKER_BUILDKIT") or "").strip().lower() in _BUILDKIT_FALSE_VALUES:
        raise RuntimeError(
            "Host-side image builds require Docker BuildKit (DOCKER_BUILDKIT must not be disabled)."
        )
    ctx_dir = Path(str(spec["context"]))
    df_path = Path(str(spec["dockerfile"]))
    if spec.get("ca_pem_file"):
        df_text = df_path.read_text(encoding="utf-8", errors="replace")
        staged_ctx = tmp_root / "staged-context"
        stage_capsem_ca_in_context(ctx_dir, staged_ctx, spec)
        ctx_dir = staged_ctx
        df_text = patch_dockerfile_for_capsem_ca(df_text)
        df_path = tmp_root / "Dockerfile.capsem"
        df_path.write_text(df_text, encoding="utf-8")
    return ctx_dir, df_path


_RESOLVED_BUILD_KEYS = frozenset(
    {
        "context",
        "dockerfile",
        "args",
        "target",
        "network",
        "docker_config_dir",
        "ca_pem_file",
        "ca_bundle_file",
    }
)


def _is_resolved_build_spec(val: Mapping[str, Any]) -> bool:
    return (
        set(val.keys()) == _RESOLVED_BUILD_KEYS
        and isinstance(val.get("context"), str)
        and isinstance(val.get("dockerfile"), str)
        and isinstance(val.get("args"), Mapping)
        and val.get("network") in ("none", "default")
        and all(
            val.get(k) is None or isinstance(val.get(k), str)
            for k in ("target", "docker_config_dir", "ca_pem_file", "ca_bundle_file")
        )
    )
