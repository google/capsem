"""Build context `.dockerignore` filtering, symlink containment, and deterministic cache keys."""

from __future__ import annotations

import fnmatch
import hashlib
import json
import os
import platform as py_platform
import stat
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any


def _is_within(child: Path, parent: Path) -> bool:
    try:
        child.relative_to(parent)
        return True
    except ValueError:
        return False


def default_linux_platform() -> str:
    """Return the target Linux OCI platform matching the current host architecture."""
    override = os.environ.get("CAPSEM_INSPECT_BUILD_PLATFORM", "").strip()
    if override:
        return override
    arch = py_platform.machine().lower()
    return "linux/arm64" if arch in ("aarch64", "arm64") else "linux/amd64"


def _parse_dockerignore(context_dir: Path) -> list[tuple[bool, str]]:
    ignore_file = context_dir / ".dockerignore"
    if ignore_file.is_symlink() and not _is_within(
        Path(os.path.realpath(ignore_file)), context_dir
    ):
        raise ValueError(f".dockerignore symlink escapes build context {context_dir}")
    if not ignore_file.is_file():
        return []
    rules: list[tuple[bool, str]] = []
    for raw_line in ignore_file.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        neg = line.startswith("!")
        pat = (line[1:].strip() if neg else line).replace("\\", "/").strip("/")
        while pat.startswith("./"):
            pat = pat[2:].strip("/")
        if pat:
            rules.append((neg, pat))
    return rules


def _matches_dockerignore_pattern(rel_posix: str, pat: str) -> bool:
    parts = rel_posix.split("/")
    for i in range(1, len(parts) + 1):
        prefix = "/".join(parts[:i])
        if fnmatch.fnmatchcase(prefix, pat):
            return True
        if pat.endswith("/**") and fnmatch.fnmatchcase(prefix, pat[:-3]):
            return True
    return False


def _is_dockerignored(rel_posix: str, rules: list[tuple[bool, str]]) -> bool:
    ignored = False
    for neg, pat in rules:
        if _matches_dockerignore_pattern(rel_posix, pat):
            ignored = not neg
    return ignored


def iter_context_files(
    context_dir: Path, *, include_ignored_regular: bool = False
) -> list[tuple[str, Path]]:
    """Return sorted `(rel_posix, path)` for context entries, enforcing symlink containment."""
    ctx_real = Path(os.path.realpath(context_dir))
    if not ctx_real.is_dir():
        raise FileNotFoundError(f"Build context directory not found: {context_dir}")
    rules = _parse_dockerignore(ctx_real)
    has_negation = any(neg for neg, _ in rules)
    entries: list[tuple[str, Path]] = []

    for root, dirnames, filenames in os.walk(ctx_real, topdown=True, followlinks=False):
        root_path = Path(root)
        rel_root = root_path.relative_to(ctx_real)
        dirnames.sort()
        filenames.sort()

        kept_dirs: list[str] = []
        for d in dirnames:
            d_path = root_path / d
            d_rel = (rel_root / d).as_posix()
            ignored_dir = _is_dockerignored(d_rel, rules)
            if d_path.is_symlink():
                if not ignored_dir and not _is_within(Path(os.path.realpath(d_path)), ctx_real):
                    raise ValueError(
                        f"Build context directory symlink {d_rel!r} escapes context root {ctx_real}"
                    )
                continue
            if ignored_dir and not has_negation and not include_ignored_regular:
                continue
            kept_dirs.append(d)
        dirnames[:] = kept_dirs

        for fname in filenames:
            fpath = root_path / fname
            f_rel = (rel_root / fname).as_posix()
            ignored_file = _is_dockerignored(f_rel, rules)
            st_mode = fpath.lstat().st_mode
            if stat.S_ISLNK(st_mode):
                if ignored_file:
                    continue
                if not _is_within(Path(os.path.realpath(fpath)), ctx_real):
                    raise ValueError(
                        f"Build context symlink {f_rel!r} escapes context root {ctx_real}"
                    )
                entries.append((f_rel, fpath))
            elif stat.S_ISREG(st_mode) and (include_ignored_regular or not ignored_file):
                entries.append((f_rel, fpath))

    return sorted(entries, key=lambda item: item[0])


def prepare_compose_build_service(
    svc: Mapping[str, Any], *, direct_config: bool = False
) -> tuple[dict[str, Any], str | None]:
    """Normalize service `build` / `dockerfile` before `compose_service.extract_compose_fields`."""
    raw_build, raw_df = svc.get("build"), svc.get("dockerfile")
    if raw_build is None and raw_df is None:
        return dict(svc), None
    stanza = "build" if raw_build is not None else "dockerfile"
    out = {k: v for k, v in svc.items() if k != "dockerfile"}
    if raw_df is not None:
        if not isinstance(raw_df, str) or not raw_df.strip():
            raise ValueError("Compose 'dockerfile' must be a non-empty path string")
        df_s, df_p = raw_df.strip(), Path(raw_df.strip())
        if raw_build is None:
            out["build"] = (
                {"context": str(df_p.parent) or ".", "dockerfile": df_p.name}
                if direct_config
                else {"context": ".", "dockerfile": df_s}
            )
        elif isinstance(raw_build, str) and raw_build.strip():
            out["build"] = {"context": raw_build.strip(), "dockerfile": df_s}
        elif isinstance(raw_build, Mapping):
            out["build"] = {"dockerfile": df_s, **dict(raw_build)}
        else:
            raise ValueError("Compose 'build' must be a context path string or mapping")
    elif isinstance(raw_build, str) and raw_build.strip():
        out["build"] = {"context": raw_build.strip()}
    elif isinstance(raw_build, Mapping):
        out["build"] = dict(raw_build)
    else:
        raise ValueError("Compose 'build' must be a non-empty context path string or mapping")

    b_map = out["build"]
    for key in ("context", "dockerfile"):
        if (
            key in b_map
            and b_map[key] is not None
            and (not isinstance(b_map[key], str) or not b_map[key].strip())
        ):
            raise ValueError(f"Compose 'build.{key}' must be a non-empty path string")
    ctx_val = str(b_map.setdefault("context", ".")).strip()
    if "://" in ctx_val or ctx_val.startswith("git@"):
        raise ValueError(f"Remote build context {ctx_val!r} is not supported by inspect-capsem")
    return out, stanza


def resolve_context_and_dockerfile(
    dockerfile: str,
    build_context: str | None,
    allowed_roots: Sequence[Path],
    *,
    stanza: str = "build",
) -> tuple[Path, Path]:
    """Validate `build_context` and `dockerfile` realpaths against `allowed_roots`."""
    df_cand = Path(dockerfile).expanduser()
    ctx_cand = Path(build_context).expanduser() if build_context is not None else df_cand.parent
    ctx_real = Path(os.path.realpath(ctx_cand))
    if not any(_is_within(ctx_real, r) for r in allowed_roots):
        raise ValueError(
            f"Build context {str(ctx_real)!r} for {stanza!r} is not allowlisted in "
            "CAPSEM_INSPECT_ALLOWED_HOST_PATHS / allowed_host_paths / allowed_contexts."
        )
    if ctx_real.exists() and not ctx_real.is_dir():
        raise ValueError(f"Build context path is not a directory: {ctx_cand}")
    if ctx_real.is_dir():
        iter_context_files(ctx_real)

    if (
        not df_cand.exists()
        and df_cand.name == "Dockerfile"
        and (ctx_real / "Containerfile").exists()
    ):
        df_cand = ctx_real / "Containerfile"
    df_real = Path(os.path.realpath(df_cand))
    if not any(_is_within(df_real, r) for r in allowed_roots):
        raise ValueError(
            f"Dockerfile {str(df_real)!r} for {stanza!r} is not allowlisted in "
            "CAPSEM_INSPECT_ALLOWED_HOST_PATHS / allowed_host_paths / allowed_contexts."
        )
    if _is_within(df_cand.absolute(), ctx_cand.absolute()) and not _is_within(df_real, ctx_real):
        raise ValueError(f"Dockerfile {str(df_cand)!r} escapes build context {str(ctx_real)!r}")
    if (df_cand.exists() or df_cand.is_symlink()) and not df_real.is_file():
        raise ValueError(f"Dockerfile path is not a regular file: {df_cand}")
    return ctx_real, df_real


def compute_build_cache_key(
    spec: Mapping[str, Any],
    platform: str | None = None,
    ca_fingerprint: str | None = None,
) -> str:
    """Compute a deterministic SHA-256 cache key over the build spec, CA bundle, and context."""
    from .build_ca import compute_ca_fingerprint

    eff_platform = (platform or default_linux_platform()).strip()
    ctx_dir = Path(str(spec["context"]))
    df_str = spec.get("dockerfile")
    if not df_str:
        raise FileNotFoundError("Dockerfile path is required in build spec")
    df_path = Path(str(df_str))
    if not df_path.is_file():
        raise FileNotFoundError(f"Dockerfile not found: {df_path}")
    df_bytes = df_path.read_bytes()
    eff_ca_fp = ca_fingerprint if ca_fingerprint is not None else compute_ca_fingerprint(spec)

    h = hashlib.sha256()
    header = {
        "v": 2,
        "platform": eff_platform,
        "dockerfile_sha256": hashlib.sha256(df_bytes).hexdigest(),
        "target": spec.get("target"),
        "args": sorted((str(k), str(v)) for k, v in (spec.get("args") or {}).items()),
        "network": spec.get("network", "none"),
        "ca_sha256": eff_ca_fp or "",
    }
    h.update(json.dumps(header, sort_keys=True, separators=(",", ":")).encode())
    for rel_posix, fpath in iter_context_files(ctx_dir, include_ignored_regular=True):
        st = fpath.lstat()
        if fpath.is_symlink():
            h.update(
                f"\0L\0{rel_posix}\0{os.readlink(fpath)}".encode("utf-8", errors="surrogateescape")
            )
        else:
            fh = hashlib.sha256(fpath.read_bytes()).hexdigest()
            h.update(f"\0F\0{rel_posix}\0{oct(st.st_mode & 0o7777)}\0{fh}".encode())
    return h.hexdigest()
