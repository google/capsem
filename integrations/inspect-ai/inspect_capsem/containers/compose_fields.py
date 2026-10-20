"""Compose service field normalizers and host-path containment helpers."""

from __future__ import annotations

import logging
import os
from collections.abc import Collection, Mapping, Sequence
from pathlib import Path, PurePosixPath
from typing import Any

from .compose_inputs import build_host_compose_inputs
from .compose_service import extract_compose_fields
from .compose_values import (
    _NAMED_VOL_RE,
    _combine_entrypoint_and_command,
    _looks_like_host_bind_source,
    _resolve_declared_volumes,
    parse_cpus_to_cpu_count,
    parse_memory_to_ram_gb,
)
from .compose_values import _normalize_environment as _normalize_compose_environment
from .compose_values import _normalize_volumes as _normalize_compose_volumes

logger = logging.getLogger(__name__)
CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR = "CAPSEM_INSPECT_ALLOWED_HOST_PATHS"
_UNSUPPORTED_KEYS = (
    "cap_add cap_drop devices security_opt sysctls pid ipc uts cgroup cgroup_parent userns_mode"
)
_REJECTED_SERVICE_KEYS: dict[str, str] = {
    **{
        k: f"Compose {k!r} is not supported in Capsem OCI-workload mode."
        for k in _UNSUPPORTED_KEYS.split()
    },
    "privileged": (
        "Compose 'privileged' is not supported in Capsem OCI-workload mode; "
        "Capsem isolates workloads via micro-VM hardware virtualization."
    ),
}


def _realpath(raw: str | Path) -> Path:
    return Path(os.path.realpath(Path(raw).expanduser()))


def _is_under_root(target: Path, root: Path) -> bool:
    return target == root or target.is_relative_to(root)


def resolve_effective_allowed_host_paths(
    task_allowed_host_paths: Sequence[str] = (),
) -> tuple[str, ...]:
    """Resolve effective host-path allowlist from operator env narrowed by task config."""
    raw_op = os.environ.get(CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR, "")
    op_roots = tuple(
        s.strip() for chunk in raw_op.split(",") for s in chunk.split(os.pathsep) if s.strip()
    )
    if not op_roots:
        return ()
    task_roots = tuple(r.strip() for r in task_allowed_host_paths if r and r.strip())
    if not task_roots:
        return op_roots
    res_op, res_task = [_realpath(r) for r in op_roots], [_realpath(r) for r in task_roots]
    narrowed = [str(tr) for tr in res_task if any(_is_under_root(tr, o) for o in res_op)]
    for opr in res_op:
        if any(_is_under_root(opr, tr) for tr in res_task) and str(opr) not in narrowed:
            narrowed.append(str(opr))
    return tuple(narrowed)


def _is_host_path_allowed(
    candidate: Path, allowed_host_paths: Sequence[str] = (), *, base_dir: Path | None = None
) -> bool:
    """Return True if `candidate`'s `realpath` stays inside `base_dir` or allowed roots."""
    resolved = _realpath(candidate)
    if base_dir is not None and _is_under_root(resolved, _realpath(base_dir)):
        return True
    return any(_is_under_root(resolved, _realpath(r)) for r in allowed_host_paths if r)


def _resolve_bind_source(
    src: str, base_dir: Path | None, allowed_host_paths: Sequence[str] = ()
) -> str:
    expanded = Path(src).expanduser()
    target = expanded
    if base_dir is not None and not expanded.is_absolute() and not src.startswith("~"):
        cand = base_dir / expanded
        if src.startswith(".") or "/" in src or cand.exists() or cand.is_symlink():
            target = cand
    resolved = target.resolve()
    if expanded.name == "docker.sock" or resolved.name == "docker.sock":
        raise ValueError(
            "Mounting the host Docker socket ('docker.sock') into a sandbox is forbidden."
        )
    eff_roots = resolve_effective_allowed_host_paths(allowed_host_paths)
    if not _is_host_path_allowed(resolved, eff_roots, base_dir=base_dir):
        raise ValueError(
            f"Host bind mount source {src!r} (resolved to {str(resolved)!r}) is outside the "
            f"Compose directory and not allowlisted via {CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR} "
            "/ allowed_host_paths."
        )
    return str(resolved)


def _normalize_environment(
    raw_env: Any,
    base_dir: Path | None = None,
    *,
    allowed_host_env: Sequence[str] = (),
    sample_metadata: Mapping[str, Any] | None = None,
) -> dict[str, str]:
    inputs = build_host_compose_inputs(
        base_dir=base_dir,
        allowed_host_env=allowed_host_env,
        sample_metadata=sample_metadata,
        include_dotenv=True,
    )
    return _normalize_compose_environment(raw_env, inputs)


def normalize_volumes(
    raw_vols: Any,
    base_dir: Path | None,
    allowed_host_paths: Sequence[str] = (),
    *,
    declared_volumes: Collection[str] = (),
) -> tuple[str, ...]:
    """Validate and resolve Compose or config bind-mount and declared named volumes."""
    if not isinstance(raw_vols, (list, tuple)) or not raw_vols:
        return ()
    specs = _normalize_compose_volumes(
        [dict(v) if isinstance(v, Mapping) else v for v in raw_vols], base_dir
    )
    out: list[str] = []
    for spec in specs:
        parts = spec.split(":")
        if len(parts) >= 2 and PurePosixPath(parts[1].strip()).name == "docker.sock":
            raise ValueError(
                "Mounting the host Docker socket ('docker.sock') into a sandbox is forbidden."
            )
        if len(parts) >= 2 and _looks_like_host_bind_source(parts[0]):
            parts[0] = _resolve_bind_source(parts[0], base_dir, allowed_host_paths)
            out.append(":".join(parts))
        elif (
            len(parts) >= 2
            and parts[0] in declared_volumes
            and _NAMED_VOL_RE.match(parts[0])
            and parts[1].strip().startswith("/")
        ):
            out.append(":".join(parts))
        else:
            raise ValueError(
                f"Named or non-bind Compose volume {spec!r} is not supported in Capsem "
                "OCI-workload mode unless declared in top-level 'volumes:'; "
                "use a relative or allowlisted (allowed_host_paths) bind mount."
            )
    return tuple(out)


def extract_capsem_compose_fields(
    parsed: Mapping[str, Any],
    base_dir: Path | None = None,
    *,
    allowed_host_env: Sequence[str] = (),
    allowed_host_paths: Sequence[str] = (),
    sample_metadata: Mapping[str, Any] | None = None,
    host_build: Any = None,
) -> dict[str, Any]:
    """Extract and validate Capsem container overrides from a parsed Compose mapping."""
    from .build_grant import prepare_compose_build_service, resolve_effective_host_build

    declared_vols = _resolve_declared_volumes(parsed.get("volumes"))
    services = parsed.get("services")
    svc: Mapping[str, Any] = {}
    svc_name, parsed_for_extract, stanza = "default", parsed, None
    if isinstance(services, Mapping) and len(services) == 1:
        raw_name, raw_svc = next(iter(services.items()))
        svc_name = str(raw_name)
        if isinstance(raw_svc, Mapping):
            svc = raw_svc
            for key, msg in _REJECTED_SERVICE_KEYS.items():
                if svc.get(key) not in (None, False, [], (), {}):
                    raise ValueError(msg)
            svc_clean, stanza = prepare_compose_build_service(
                {k: v for k, v in svc.items() if k not in ("cpus", "platform")}
            )
            parsed_for_extract = {**parsed, "services": {svc_name: svc_clean}}
    inputs = build_host_compose_inputs(
        base_dir=base_dir,
        allowed_host_env=allowed_host_env,
        sample_metadata=sample_metadata,
        include_dotenv=True,
    )
    out = extract_compose_fields(parsed_for_extract, base_dir=base_dir, inputs=inputs)
    net_mode, explicit_net = out.pop("network_mode", None), svc.get("network_mode")
    if isinstance(explicit_net, str) and (cleaned_mode := explicit_net.strip()):
        if cleaned_mode not in ("bridge", "default"):
            raise ValueError(
                f"Compose network_mode {cleaned_mode!r} is not supported by inspect-capsem: "
                "per-VM network mode / air-gapped egress is not yet wired in the Capsem 0.7 "
                "ProvisionRequest contract."
            )
    elif net_mode == "none":
        raise ValueError(
            "Compose 'internal: true' networks are not supported by inspect-capsem: "
            "per-VM air-gapped egress is not yet wired in the Capsem 0.7 ProvisionRequest contract."
        )
    if out.pop("ports", None):
        raise ValueError("Compose 'ports' is not supported in Capsem OCI-workload mode.")
    out.pop("expose", None)
    if out.pop("init", None):
        logger.warning(
            "Ignoring Compose 'init: true' in service %r: Capsem OCI-workload mode supervises "
            "the container process directly.",
            svc_name,
        )
    if (
        cmd := _combine_entrypoint_and_command(
            out.pop("entrypoint", None), out.pop("command", None)
        )
    ) is not None:
        out["command"] = cmd
    if "mem_limit" in out:
        out["ram_gb"] = parse_memory_to_ram_gb(out["mem_limit"])
    d: Any = svc
    for k in ("deploy", "resources", "limits"):
        d = d.get(k) if isinstance(d, Mapping) else None
    cpus_raw = svc.get("cpus")
    if cpus_raw is None and isinstance(d, Mapping):
        cpus_raw = d.get("cpus")
    if cpus_raw is not None:
        out["cpu_count"] = parse_cpus_to_cpu_count(cpus_raw)
    if raw_vols := out.pop("volumes", None):
        out["volumes"] = normalize_volumes(
            raw_vols, base_dir, allowed_host_paths, declared_volumes=declared_vols
        )
    if stanza is not None and "dockerfile" in out:
        out["build"] = resolve_effective_host_build(
            dockerfile=out.pop("dockerfile"),
            build_context=out.pop("build_context", None),
            build_args=out.pop("build_args", None),
            build_target=out.pop("build_target", None),
            stanza=stanza,
            allowed_host_paths=allowed_host_paths,
            host_build=host_build,
        )
    return out
