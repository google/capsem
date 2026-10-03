"""Host-side image cache, build context packing, and archive staging tests."""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

import inspect_capsem._compose as compose_mod
import inspect_capsem.containers.dockerfile as dockerfile_mod
import inspect_capsem.containers.image_cache as cache_mod
import inspect_capsem.containers.runtime as runtime_mod
import pytest
from inspect_capsem._controller import CommandResult
from inspect_capsem.config import CapsemSandboxConfig

from ..conftest import Scripted, fail, ok, run_init


def test_dockerfile_image_cache_reuses_across_samples_and_recovers_on_corrupt_cache(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.delenv("INSPECT_CAPSEM_IMAGE_CACHE_DIR", raising=False)
    monkeypatch.delenv("INSPECT_CAPSEM_IMAGE_CACHE", raising=False)
    assert cache_mod._image_cache_dir() == tmp_path / ".cache" / "inspect-capsem" / "images"
    assert cache_mod._is_image_cache_enabled()

    cache_dir = tmp_path / "img-cache"
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE_DIR", str(cache_dir))
    ctx_dir = tmp_path / "ctx"
    ctx_dir.mkdir()
    dockerfile = ctx_dir / "Dockerfile"
    dockerfile.write_text("FROM debian:12\nRUN echo cached\n")
    cfg = CapsemSandboxConfig(dockerfile=str(dockerfile))

    # Sample 1 (no CA) builds the image and saves it to the host cache under digest_noca.
    ctrl1 = Scripted(
        [(cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n")), ("docker run -d", ok("c1\n"))]
    )
    ctrl1.files["/root/capsem-image-cache.tar.gz"] = b"cached-noca-bytes"
    run_init(ctrl1, cfg)
    assert sum(c.startswith("docker build") for c in ctrl1.commands) == 1
    cached_files = list(cache_dir.glob("*.tar.gz"))
    assert len(cached_files) == 1 and cached_files[0].read_bytes() == b"cached-noca-bytes"

    # Sample 2 in a CA-ready VM does NOT reuse the CA-less cached archive; it builds a CA-patched image.
    ctrl_ca = Scripted(
        [
            (dockerfile_mod._CA_READY, ok(f"{dockerfile_mod._CA_READY}\n")),
            (cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n")),
            ("docker run -d", ok("c-ca\n")),
        ]
    )
    ctrl_ca.files["/root/capsem-image-cache.tar.gz"] = b"cached-ca-bytes"
    run_init(ctrl_ca, cfg)
    assert sum(c.startswith("docker build") for c in ctrl_ca.commands) == 1
    assert len(list(cache_dir.glob("*.tar.gz"))) == 2

    # Sample 3 in a fresh no-CA VM loads the no-CA cached image archive and skips `docker build`.
    ctrl2 = Scripted([("docker run -d", ok("c2\n"))])
    run_init(ctrl2, cfg)
    assert not any(c.startswith("docker build") for c in ctrl2.commands)
    expected_guest_archive = cache_mod._image_archive_guest_path(cached_files[0])
    assert ctrl2.uploads[expected_guest_archive] == b"cached-noca-bytes"
    assert any("docker load" in c for c in ctrl2.commands)

    # If upload_to_vm or download_from_vm raises, sample falls back to building without failing.
    class FailingTransfer(Scripted):
        async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
            if guest_path.startswith("/root/capsem-image-cache-"):
                raise RuntimeError("upload timeout")
            await super().upload_to_vm(vm_id, guest_path, data)

        async def download_from_vm(self, vm_id: str, guest_path: str) -> bytes:
            if guest_path.startswith("/root/capsem-image-cache-"):
                raise RuntimeError("download timeout")
            return await super().download_from_vm(vm_id, guest_path)

    ctrl_err = FailingTransfer(
        [
            (cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n")),
            ("docker run -d", ok("c-err\n")),
        ]
    )
    run_init(ctrl_err, cfg)
    assert sum(c.startswith("docker build") for c in ctrl_err.commands) == 1
    assert list(cache_dir.glob("*.tmp")) == []

    # Transient `docker load` failure (daemon busy / disk full) preserves the cache file and rebuilds.
    ctrl_transient = Scripted(
        [
            ("docker load", fail(stderr="Cannot connect to the Docker daemon")),
            ("docker run -d", ok("c-trans\n")),
        ]
    )
    run_init(ctrl_transient, cfg)
    assert sum(c.startswith("docker build") for c in ctrl_transient.commands) == 1
    assert cached_files[0].exists()

    # Corrupt archive failure unlinks the cache file and rebuilds.
    ctrl3 = Scripted(
        [
            ("docker load", fail(stderr="corrupt archive: unexpected EOF")),
            ("docker run -d", ok("c3\n")),
        ]
    )
    run_init(ctrl3, cfg)
    assert sum(c.startswith("docker build") for c in ctrl3.commands) == 1
    assert not cached_files[0].exists()

    # Concurrent _build_dockerfile_in_vm with cache enabled serializes via _IMAGE_BUILD_LOCKS so the
    # second build loads the cached archive instead of running a duplicate `docker build`.
    ctrl_conc = Scripted([(cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n"))])
    ctrl_conc.files["/root/capsem-image-cache.tar.gz"] = b"cached-conc-bytes"

    async def _build_twice() -> tuple[str, str]:
        spec = cfg.to_container_spec()
        return await asyncio.gather(
            runtime_mod._build_dockerfile_in_vm(ctrl_conc, "vm-c1", spec),
            runtime_mod._build_dockerfile_in_vm(ctrl_conc, "vm-c2", spec),
        )

    t1, t2 = asyncio.run(_build_twice())
    assert t1 == t2
    assert sum(c.startswith("docker build") for c in ctrl_conc.commands) == 1

    # Disabling cache via INSPECT_CAPSEM_IMAGE_CACHE=0 bypasses cache load/save.
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE", "0")
    assert not cache_mod._is_image_cache_enabled()


def test_pack_build_context_and_archive_staging(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import io
    import tarfile

    from inspect_capsem.containers.image_cache import (
        _pack_path_tar_gz,
        _stage_archive_to_vm_dir,
        pack_build_context,
    )
    from inspect_capsem.containers.runtime import (
        _build_docker_run_command,
        _healthcheck_shell_command,
        _healthcheck_timings,
        _parse_healthcheck_duration_secs,
        _stage_bind_volumes,
        start_container_for_init,
    )

    ctx = tmp_path / "ctx"
    ctx.mkdir()
    (ctx / "a.txt").write_text("a")
    ext_df = tmp_path / "External.Dockerfile"
    ext_df.write_text("FROM scratch\n")
    tar_bytes, rel_df = pack_build_context(ctx, ext_df)
    assert rel_df == ".capsem.Dockerfile" and len(tar_bytes) > 0

    # Differing file/dir mtimes produce byte-for-byte identical tar.gz archives.
    os.utime(ctx / "a.txt", (1000.0, 1000.0))
    os.utime(ext_df, (1000.0, 1000.0))
    tar_bytes_t1, _ = pack_build_context(ctx, ext_df)
    os.utime(ctx / "a.txt", (999999.0, 999999.0))
    os.utime(ext_df, (888888.0, 888888.0))
    tar_bytes_t2, _ = pack_build_context(ctx, ext_df)
    assert tar_bytes_t1 == tar_bytes_t2

    # .dockerignore patterns (comments, blank lines, exclusions, !negation, keep_rel_paths for Dockerfile).
    ig_ctx = tmp_path / "ig_ctx"
    ig_ctx.mkdir()
    (ig_ctx / "Dockerfile").write_text("FROM alpine:3.20\n")
    (ig_ctx / ".dockerignore").write_text(
        "# comment line\n"
        "\n"
        "!\n"
        "/\n"
        "./dot_prefixed.txt\n"
        "node_modules\n"
        "*.log\n"
        "!important.log\n"
        "/root_only.txt\n"
        "**/deep.ignore\n"
        "a/**/b\n"
        "sub/*.tmp\n"
        "!sub/keep.tmp\n"
        "[!k]har.txt\n"
        "Dockerfile\n"
    )
    (ig_ctx / "keep.txt").write_text("keep")
    (ig_ctx / "dot_prefixed.txt").write_text("drop-dot")
    (ig_ctx / "debug.log").write_text("drop")
    (ig_ctx / "important.log").write_text("keep-log")
    (ig_ctx / "root_only.txt").write_text("drop-root")
    (ig_ctx / "char.txt").write_text("drop-char")
    (ig_ctx / "khar.txt").write_text("keep-char")
    nm = ig_ctx / "node_modules"
    nm.mkdir()
    (nm / "pkg.js").write_text("drop-nm")
    a_dir = ig_ctx / "a"
    a_dir.mkdir()
    (a_dir / "b").write_text("drop-ab")
    sub = ig_ctx / "sub"
    sub.mkdir()
    (sub / "x.log").write_text("keep-sub-log")
    (sub / "root_only.txt").write_text("keep-in-sub")
    (sub / "deep.ignore").write_text("drop-deep")
    (sub / "drop.tmp").write_text("drop")
    (sub / "keep.tmp").write_text("keep")
    ig_tar, ig_rel_df = pack_build_context(ig_ctx, ig_ctx / "Dockerfile")
    assert ig_rel_df == "Dockerfile"
    with tarfile.open(fileobj=io.BytesIO(ig_tar), mode="r:gz") as tf:
        names = set(tf.getnames())
    assert "Dockerfile" in names
    assert "keep.txt" in names
    assert "important.log" in names
    assert "khar.txt" in names
    assert "sub/x.log" in names
    assert "sub/root_only.txt" in names
    assert "sub/keep.tmp" in names
    assert "dot_prefixed.txt" not in names
    assert "debug.log" not in names
    assert "root_only.txt" not in names
    assert "char.txt" not in names
    assert "a/b" not in names
    assert "sub/deep.ignore" not in names
    assert "sub/drop.tmp" not in names
    assert not any("node_modules" in n for n in names)

    # Build context size cap (INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES) applies to both pack_build_context
    # and _stage_bind_volumes, while bind volumes still preserve mtime.
    monkeypatch.setenv("INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES", "5")
    assert cache_mod._max_build_context_bytes() == 5
    big_file = tmp_path / "big.bin"
    big_file.write_bytes(b"0123456789")
    os.utime(big_file, (12345.0, 12345.0))
    with pytest.raises(ValueError, match="exceeds maximum size"):
        _pack_path_tar_gz(big_file, max_bytes=5)
    with pytest.raises(ValueError, match="exceeds maximum size"):
        _pack_path_tar_gz(ig_ctx, max_bytes=5)
    with pytest.raises(ValueError, match=r"Build context .* exceeds maximum size"):
        pack_build_context(ctx, big_file)
    with pytest.raises(ValueError, match=r"Bind mount .* exceeds maximum size"):
        asyncio.run(_stage_bind_volumes(Scripted(), "vm-1", (f"{big_file}:/mnt/big.bin",)))
    vol_tar = _pack_path_tar_gz(big_file)
    with tarfile.open(fileobj=io.BytesIO(vol_tar), mode="r:gz") as tf:
        assert int(tf.getmember("big.bin").mtime) == 12345
    monkeypatch.setenv("INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES", "0")
    assert cache_mod._max_build_context_bytes() == 0
    assert len(pack_build_context(ctx, big_file)[0]) > 0
    monkeypatch.setenv("INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES", "invalid")
    assert cache_mod._max_build_context_bytes() == cache_mod._MAX_BUILD_CONTEXT_BYTES
    monkeypatch.delenv("INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES", raising=False)

    xfer_ok = Scripted()
    asyncio.run(_stage_archive_to_vm_dir(xfer_ok, "vm-1", tar_bytes, "/tmp/b"))
    assert "/tmp/b.tar.gz" in xfer_ok.uploads

    xfer_fail = Scripted([("tar -xzf", fail(stderr="corrupt"))])
    with pytest.raises(RuntimeError, match="corrupt"):
        asyncio.run(_stage_archive_to_vm_dir(xfer_fail, "vm-1", tar_bytes, "/tmp/b"))

    host_file = tmp_path / "single.txt"
    host_file.write_text("single")
    empty_a = tmp_path / "empty_a"
    empty_b = tmp_path / "empty_b"
    empty_a.mkdir()
    empty_b.mkdir()
    # Even if a relative dir `pgdata` exists in cwd, bare named volume `pgdata:/var/lib/pg` is untouched.
    monkeypatch.chdir(tmp_path)
    (tmp_path / "pgdata").mkdir()
    xfer_ok.uploads.clear()
    vols = asyncio.run(
        _stage_bind_volumes(
            xfer_ok,
            "vm-1",
            (
                "/anon",
                "pgdata:/var/lib/pg",
                f"{tmp_path / 'missing'}:/mnt/missing",
                f"{ctx}:/mnt/ctx:ro",
                f"{host_file}:/mnt/single.txt",
                f"{empty_a}:/mnt/a",
                f"{empty_b}:/mnt/b",
            ),
        )
    )
    assert vols[0] == "/anon"
    assert vols[1] == "pgdata:/var/lib/pg"
    assert vols[2] == f"{tmp_path / 'missing'}:/mnt/missing"
    assert vols[3].endswith(":/mnt/ctx:ro")
    assert vols[4].endswith("/single.txt:/mnt/single.txt")
    # Two empty host dirs with identical tar bytes get distinct guest stage dirs.
    assert vols[5].split(":")[0] != vols[6].split(":")[0]

    cmd1 = _build_docker_run_command(
        "c1",
        "img:1",
        CapsemSandboxConfig(
            init=True,
            mem_limit="1g",
            network_mode="host",
            user="root",
            environment={"A": "1"},
            ports=("80:80",),
            expose=("90",),
            entrypoint=("/ep", "--flag"),
            command="echo ok",
        ).to_container_spec(),
        ["/h:/g"],
    )
    assert "--init" in cmd1 and "--memory 1g" in cmd1 and "--entrypoint /ep" in cmd1
    assert "echo ok" in cmd1 and "sh -c" not in cmd1 and "-v /h:/g" in cmd1

    cmd2 = _build_docker_run_command(
        "c2",
        "img:2",
        CapsemSandboxConfig(entrypoint="/ep", command=("arg1", "arg2")).to_container_spec(),
        [],
    )
    assert "--entrypoint /ep" in cmd2 and "arg1 arg2" in cmd2

    cmd3 = _build_docker_run_command(
        "c3", "img:3", CapsemSandboxConfig(entrypoint=()).to_container_spec(), []
    )
    assert "sleep infinity" not in cmd3

    # -w only: a bind mount of the empty VM dir would mask the image's /testbed.
    cmd4 = _build_docker_run_command(
        "c4", "img:4", CapsemSandboxConfig(working_dir="/testbed").to_container_spec(), []
    )
    assert "-w /testbed" in cmd4 and "/testbed:/testbed" not in cmd4 and " -v " not in cmd4

    assert _healthcheck_shell_command({"disable": True}) == (None, 0, 0)
    assert _healthcheck_shell_command({"test": "NONE"}) == (None, 0, 0)
    assert _healthcheck_shell_command({"test": "curl -f http://localhost"}) == (
        "curl -f http://localhost",
        30,
        1,
    )
    assert _healthcheck_shell_command({"test": ["NONE"]}) == (None, 0, 0)
    assert _healthcheck_shell_command(
        {"test": ["CMD-SHELL", "test -f /ok"], "retries": 2, "interval": "500ms"}
    ) == ("test -f /ok", 2, 1)
    assert _healthcheck_shell_command(
        {"test": ["CMD", "curl", "http://localhost"], "interval": "2s"}
    ) == ("curl http://localhost", 30, 2)
    assert _healthcheck_shell_command({"test": ["curl", "localhost"]}) == ("curl localhost", 30, 1)
    assert _healthcheck_shell_command({"test": 123}) == (None, 0, 0)

    # Duration parsing and healthcheck timeout + start_period semantics.
    assert _parse_healthcheck_duration_secs(None, default=5) == 5
    assert _parse_healthcheck_duration_secs("", default=5) == 5
    assert _parse_healthcheck_duration_secs(0, default=5, min_secs=0) == 0
    assert _parse_healthcheck_duration_secs("0ms", default=5, min_secs=0) == 0
    assert _parse_healthcheck_duration_secs("500ms", default=5, min_secs=0) == 1
    assert _parse_healthcheck_duration_secs("500us", default=5) == 1
    assert _parse_healthcheck_duration_secs("500µs", default=5) == 1
    assert _parse_healthcheck_duration_secs("1500000000ns", default=5) == 1
    assert _parse_healthcheck_duration_secs("1.5s", default=5) == 1
    assert _parse_healthcheck_duration_secs("1m30s", default=5) == 90
    assert _parse_healthcheck_duration_secs("2m", default=5) == 120
    assert _parse_healthcheck_duration_secs("1h", default=5) == 3600
    assert _parse_healthcheck_duration_secs("4", default=5) == 4
    with pytest.raises(ValueError, match="Invalid healthcheck duration"):
        _parse_healthcheck_duration_secs("bad", default=5)
    assert _healthcheck_timings(
        {"test": "true", "retries": 2, "interval": "1s", "timeout": "7s", "start_period": "10s"}
    ) == (7, 10)

    hc_ctrl = Scripted([("docker run -d", ok("cid-hc\n"))])
    asyncio.run(
        start_container_for_init(
            hc_ctrl,
            "vm-1",
            CapsemSandboxConfig(
                image="alpine:3.20",
                healthcheck={
                    "test": "check_ready",
                    "retries": 2,
                    "interval": "1s",
                    "timeout": "9s",
                    "start_period": "15s",
                },
            ).to_container_spec(),
        )
    )
    hc_wait = next(c for c in hc_ctrl.commands if "_hc_deadline=" in c)
    assert "timeout 9 docker exec cid-hc sh -c check_ready" in hc_wait


def test_build_timeout_and_exec_failure_formatting(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Verify build_timeout config and dual stderr/stdout failure messages."""
    assert CapsemSandboxConfig().build_timeout == 3600

    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text("services:\n  web:\n    image: alpine:3.20\n")
    resolved_cfg = compose_mod.resolve_compose_file(
        CapsemSandboxConfig(compose_file=str(compose_path), build_timeout=2100)
    )
    assert resolved_cfg.build_timeout == 2100

    # _format_exec_failure includes both stderr and stdout when both are non-empty, and truncates long tails.
    both_res = CommandResult(
        exit_code=101,
        stdout="Step 4/8 : RUN cargo build\nerror[E0432]: unresolved import `foo`\n",
        stderr="The command '/bin/sh -c cargo build' returned a non-zero code: 101\n",
    )
    formatted = cache_mod._format_exec_failure(both_res)
    assert "exit_code=101" in formatted
    assert "stderr:\nThe command" in formatted
    assert "stdout:\nStep 4/8" in formatted and "unresolved import `foo`" in formatted

    long_lines = "\n".join(f"line-{i}" for i in range(100))
    trunc_res = CommandResult(exit_code=1, stdout=long_lines, stderr="")
    trunc_fmt = cache_mod._format_exec_failure(trunc_res, max_lines=5, max_chars=40)
    assert trunc_fmt.startswith("... [truncated]\n")
    assert cache_mod._format_exec_failure(CommandResult(2, "", "")) == "exit_code=2 (no output)"

    # Verify _build_dockerfile_in_vm passes resolved build_timeout and formats both streams on failure.
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE", "0")
    df_file = tmp_path / "Dockerfile"
    df_file.write_text("FROM debian:12\nRUN cargo test\n")
    seen_timeouts: list[int] = []

    class BuildFailCtrl(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            if command.startswith("docker build"):
                seen_timeouts.append(timeout)
                return CommandResult(
                    exit_code=1,
                    stdout="cargo error: failed to compile crate\n",
                    stderr="The command '/bin/sh -c cargo test' returned a non-zero code: 1\n",
                )
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    with pytest.raises(RuntimeError) as exc_info:
        asyncio.run(
            runtime_mod._build_dockerfile_in_vm(
                BuildFailCtrl(),
                "vm-b",
                CapsemSandboxConfig(
                    dockerfile=str(df_file), build_timeout=2700
                ).to_container_spec(),
            )
        )
    assert seen_timeouts == [2700]
    err_msg = str(exc_info.value)
    assert "cargo error: failed to compile crate" in err_msg
    assert "returned a non-zero code: 1" in err_msg


def test_image_cache_size_caps_and_lru_pruning(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Skip caching oversized images and prune cache dir to max_total_bytes."""
    cache_dir = tmp_path / "img-cache"
    cache_dir.mkdir()
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE_DIR", str(cache_dir))
    monkeypatch.setenv("INSPECT_CAPSEM_MAX_CACHE_IMAGE_BYTES", "1000")
    monkeypatch.setenv("INSPECT_CAPSEM_MAX_CACHE_TOTAL_BYTES", "250")
    assert cache_mod._max_cache_image_bytes() == 1000
    assert cache_mod._max_cache_total_bytes() == 250

    # 1. Image exceeding max_img_bytes in VM emits CAPSEM_IMAGE_TOO_LARGE and is not downloaded.
    ctrl_huge = Scripted(
        [(cache_mod._IMAGE_TOO_LARGE, ok(f"{cache_mod._IMAGE_TOO_LARGE}:9999999\n"))]
    )
    out_huge = cache_dir / "huge.tar.gz"
    asyncio.run(cache_mod._save_built_image_to_cache(ctrl_huge, "vm-1", "tag:1", out_huge))
    assert not out_huge.exists()

    # 2. LRU pruning evicts oldest .tar.gz archives while preserving the newly written archive.
    old1 = cache_dir / "old1.tar.gz"
    old2 = cache_dir / "old2.tar.gz"
    old1.write_bytes(b"a" * 120)
    old2.write_bytes(b"b" * 120)
    os.utime(old1, (1000.0, 1000.0))
    os.utime(old2, (2000.0, 2000.0))

    ctrl_ok = Scripted([(cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n"))])
    ctrl_ok.files["/root/capsem-image-cache.tar.gz"] = b"c" * 100
    newest = cache_dir / "newest.tar.gz"
    asyncio.run(cache_mod._save_built_image_to_cache(ctrl_ok, "vm-1", "tag:2", newest))
    assert newest.is_file()
    assert not old1.exists()
    assert old2.is_file()
    assert sum(p.stat().st_size for p in cache_dir.glob("*.tar.gz")) <= 250

    # Zero cap disables saving completely.
    monkeypatch.setenv("INSPECT_CAPSEM_MAX_CACHE_IMAGE_BYTES", "0")
    skipped = cache_dir / "skipped.tar.gz"
    asyncio.run(cache_mod._save_built_image_to_cache(ctrl_ok, "vm-1", "tag:3", skipped))
    assert not skipped.exists()
