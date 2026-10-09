"""Unit tests for `build_context` (`.dockerignore`, symlink containment, and cache keys)."""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from inspect_capsem.containers import build_context as bc_mod
from inspect_capsem.containers import build_grant as bg_mod


def test_dockerignore_symlink_containment_and_cache_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    ctx_dir, outside = tmp_path / "ctx", tmp_path / "outside"
    ctx_dir.mkdir()
    outside.mkdir()
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(ctx_dir))
    secret_file = outside / "secret.txt"
    secret_file.write_text("top-secret", encoding="utf-8")
    (ctx_dir / "Containerfile").write_text("FROM scratch\n", encoding="utf-8")
    (ctx_dir / "kept.txt").write_text("v1", encoding="utf-8")
    (ctx_dir / "internal_link.txt").symlink_to("kept.txt")
    (ctx_dir / "ignored.log").write_text("log-v1", encoding="utf-8")
    sub = ctx_dir / "sub"
    sub.mkdir()
    (sub / "deep.txt").write_text("deep", encoding="utf-8")
    (ctx_dir / ".dockerignore").write_text(
        "# comment\n\n./*.log\n/sub/**\n!sub/deep.txt\nignored_escape_link\nignored_dir_link\n",
        encoding="utf-8",
    )
    (ctx_dir / "ignored_escape_link").symlink_to(secret_file)
    (ctx_dir / "ignored_dir_link").symlink_to(outside)

    spec = bg_mod.resolve_direct_host_build(
        {"build": {"context": str(ctx_dir), "args": ["A=1"], "target": "prod"}}
    )
    assert spec is not None and spec["dockerfile"] == str((ctx_dir / "Containerfile").resolve())
    assert spec["args"] == {"A": "1"} and spec["target"] == "prod"
    k1 = bc_mod.compute_build_cache_key(spec, platform="linux/amd64")
    assert "ignored.log" not in dict(bc_mod.iter_context_files(ctx_dir))
    assert "ignored.log" in dict(bc_mod.iter_context_files(ctx_dir, include_ignored_regular=True))
    (ctx_dir / "ignored.log").write_text("log-v2-modified", encoding="utf-8")
    k2 = bc_mod.compute_build_cache_key(spec, platform="linux/amd64")
    assert k2 != k1
    assert bc_mod.compute_build_cache_key(spec, platform="linux/amd64", ca_fingerprint="fp1") != k2
    assert bc_mod.compute_build_cache_key(
        spec, platform="linux/amd64", ca_fingerprint="fp1"
    ) != bc_mod.compute_build_cache_key(spec, platform="linux/amd64", ca_fingerprint="fp2")
    (sub / "deep.txt").write_text("deep-modified", encoding="utf-8")
    assert bc_mod.compute_build_cache_key(spec, platform="linux/amd64") != k2

    for link_name, target in (("escape_file.txt", secret_file), ("escape_dir", outside)):
        link_p = ctx_dir / link_name
        link_p.symlink_to(target)
        with pytest.raises(ValueError, match=link_name):
            bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})
        link_p.unlink()

    # Ignored directory without negation rules skips subtree walk unless include_ignored_regular=True,
    # and non-regular files (e.g. FIFOs) inside ignored directories are skipped without blocking.
    (ctx_dir / ".dockerignore").write_text(
        "sub\nignored_dir_link\nignored_escape_link\n", encoding="utf-8"
    )
    fifo_path = sub / "pipe"
    os.mkfifo(fifo_path)
    assert "sub/deep.txt" not in dict(bc_mod.iter_context_files(ctx_dir))
    incl_entries = dict(bc_mod.iter_context_files(ctx_dir, include_ignored_regular=True))
    assert "sub/deep.txt" in incl_entries
    assert "sub/pipe" not in incl_entries
    assert bc_mod.compute_build_cache_key(spec, platform="linux/amd64")
    fifo_path.unlink()

    # .dockerignore symlink escaping context is rejected
    (ctx_dir / ".dockerignore").unlink()
    (ctx_dir / ".dockerignore").symlink_to(secret_file)
    with pytest.raises(ValueError, match=r"\.dockerignore symlink escapes"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})
    (ctx_dir / ".dockerignore").unlink()
    (ctx_dir / "ignored_escape_link").unlink()
    (ctx_dir / "ignored_dir_link").unlink()

    # Dockerfile containment and file type checks
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", f"{ctx_dir},{outside}")
    (ctx_dir / ".dockerignore").write_text("Dockerfile\n", encoding="utf-8")
    df_link = ctx_dir / "Dockerfile"
    df_link.symlink_to(secret_file)
    with pytest.raises(ValueError, match="escapes build context"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})
    df_link.unlink()

    with pytest.raises(ValueError, match="not a regular file"):
        bg_mod.resolve_direct_host_build({"build": {"context": str(ctx_dir), "dockerfile": "sub"}})
    with pytest.raises(ValueError, match="not a directory"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir / "kept.txt")})

    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(ctx_dir))
    with pytest.raises(ValueError, match=r"Dockerfile .* is not allowlisted"):
        bg_mod.resolve_direct_host_build(
            {"build": {"context": str(ctx_dir), "dockerfile": str(secret_file)}}
        )

    # Platform detection and missing path errors
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_PLATFORM", "linux/arm64")
    assert bc_mod.default_linux_platform() == "linux/arm64"
    monkeypatch.delenv("CAPSEM_INSPECT_BUILD_PLATFORM", raising=False)
    for m_arch, exp_plat in (("aarch64", "linux/arm64"), ("x86_64", "linux/amd64")):
        monkeypatch.setattr(bc_mod.py_platform, "machine", lambda m=m_arch: m)
        assert bc_mod.default_linux_platform() == exp_plat
    with pytest.raises(FileNotFoundError, match="Dockerfile path is required"):
        bc_mod.compute_build_cache_key({"context": str(ctx_dir)})
    with pytest.raises(FileNotFoundError, match="Dockerfile not found"):
        bc_mod.compute_build_cache_key({"context": str(ctx_dir), "dockerfile": str(ctx_dir / "No")})
    with pytest.raises(FileNotFoundError, match="Build context directory not found"):
        bc_mod.compute_build_cache_key(
            {"context": str(tmp_path / "no_dir"), "dockerfile": str(ctx_dir / "Containerfile")}
        )
