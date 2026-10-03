"""Dockerfile CA patching, heredoc lowering, and workdir/user resolution tests."""

from __future__ import annotations

import asyncio
import gzip
import os
import subprocess
from pathlib import Path
from typing import Any, cast

import inspect_capsem._compose as compose_mod
import inspect_capsem.containers.dockerfile as dockerfile_mod
import inspect_capsem.containers.image_cache as cache_mod
import inspect_capsem.containers.runtime as runtime_mod
import pytest
from inspect_capsem.config import CapsemSandboxConfig

from ..conftest import Scripted, fail, ok, run_init

CA_LINES = list(dockerfile_mod._CA_STAGE_LINES)


CA_UPDATE = dockerfile_mod._CA_UPDATE_LINE


def test_ca_patch_multistage_scratch_and_continuations() -> None:
    patched = dockerfile_mod.patch_dockerfile_for_capsem_ca(
        "# syntax comment\n"
        "FROM golang:1.22 AS Build\n"
        "RUN go mod download\n"
        "from --platform=linux/amd64 \\\n"
        "    python:3.12 AS tools\n"
        'RUN ["pip", "install", "x"]\n'
        "FROM build AS test\n"
        "FROM scratch\n"
        "COPY --from=build /out /out\n"
    ).splitlines()
    assert patched[:2] == ["# syntax comment", "FROM golang:1.22 AS Build"]
    # A stage with a shell-form RUN also merges the CA into its system store.
    assert patched[2:6] == [*CA_LINES, CA_UPDATE]
    idx = patched.index("    python:3.12 AS tools")
    # Exec-form RUN only: the base may have no shell, so no update RUN.
    assert patched[idx + 1 : idx + 5] == [*CA_LINES, 'RUN ["pip", "install", "x"]']
    # Stages built from an earlier stage inherit the CA; scratch gets nothing.
    assert patched[-3:] == ["FROM build AS test", "FROM scratch", "COPY --from=build /out /out"]
    assert sum(line == CA_LINES[0] for line in patched) == 2
    assert sum(line == CA_UPDATE for line in patched) == 1


def test_ca_patch_keeps_image_bundle() -> None:
    patched = dockerfile_mod.patch_dockerfile_for_capsem_ca("FROM alpine:3.20\nRUN apk add curl\n")
    assert patched.splitlines() == ["FROM alpine:3.20", *CA_LINES, CA_UPDATE, "RUN apk add curl"]
    assert "/etc/ssl/certs/ca-certificates.crt" not in patched
    assert "NODE_EXTRA_CA_CERTS=/usr/local/share/ca-certificates/capsem-ca.crt" in patched
    assert dockerfile_mod.patch_dockerfile_for_capsem_ca("FROM scratch\n") == "FROM scratch\n"
    distroless = "FROM gcr.io/distroless/python3\nCOPY app /app\n"
    assert CA_UPDATE not in dockerfile_mod.patch_dockerfile_for_capsem_ca(distroless)


def test_ca_patch_ignores_heredoc_and_continuation_bodies() -> None:
    heredoc = (
        "FROM python:3.12\n"
        "RUN python3 - <<EOF\n"
        "from pathlib import Path\n"
        "print(Path('.'))\n"
        "EOF\n"
        "COPY <<-'CONF' /etc/app.conf\n"
        "\tfrom = here\n"
        "\tCONF\n"
        "RUN echo $((1<<2)) && cat <<< 'from x'\n"
    )
    assert dockerfile_mod.patch_dockerfile_for_capsem_ca(heredoc).splitlines() == [
        "FROM python:3.12",
        *CA_LINES,
        CA_UPDATE,
        *heredoc.splitlines()[1:],
    ]
    continued = 'FROM python:3.12\nRUN python -c "\\\nfrom os import path; print(path.sep)"\n'
    assert dockerfile_mod.patch_dockerfile_for_capsem_ca(continued).splitlines() == [
        "FROM python:3.12",
        *CA_LINES,
        CA_UPDATE,
        *continued.splitlines()[1:],
    ]


def test_dockerfile_build_uses_patched_copy_when_ca_ready(tmp_path: Path) -> None:
    dockerfile = tmp_path / "Dockerfile"
    dockerfile.write_text("FROM debian:12 AS base\nFROM base\n")
    ctrl = Scripted(
        [
            (dockerfile_mod._CA_READY, ok(f"{dockerfile_mod._CA_READY}\n")),
            ("docker run -d", ok("c\n")),
        ]
    )
    run_init(ctrl, CapsemSandboxConfig(dockerfile=str(dockerfile)))
    staged = gzip.decompress(ctrl.uploads["/tmp/capsem-build-ca.tar.gz"])
    assert b"FROM debian:12 AS base\n" + CA_LINES[0].encode() in staged
    build = next(c for c in ctrl.commands if c.startswith("docker build"))
    assert "--network=host" in build
    assert "-f /tmp/capsem-build-ca/Dockerfile /tmp/capsem-build" in build

    ctrl = Scripted([("docker run -d", ok("c\n"))])
    run_init(ctrl, CapsemSandboxConfig(dockerfile=str(dockerfile)))
    assert "/tmp/capsem-build-ca.tar.gz" not in ctrl.uploads
    assert any("-f /tmp/capsem-build/Dockerfile " in c for c in ctrl.commands)


def test_ca_prep_failure_is_surfaced(tmp_path: Path) -> None:
    dockerfile = tmp_path / "Dockerfile"
    dockerfile.write_text("FROM debian:12\n")
    ctrl = Scripted([(dockerfile_mod._CA_READY, fail(stderr="cat: bundle: No such file"))])
    with pytest.raises(RuntimeError, match=r"Capsem CA.*No such file"):
        run_init(ctrl, CapsemSandboxConfig(dockerfile=str(dockerfile)))
    assert not any(c.startswith("docker build") for c in ctrl.commands)
    assert ctrl.stopped == ["vm-s"]


def test_ca_patcher_edge_cases_escape_quoted_heredoc_and_derived_stage(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Backtick escape directive, quoted `<<EOF`, bitshifts (`$(( 1 << shift ))`, `$((16<<shift))`,
    # `(( x = 1 << count ))`, `let a=16<<shift`), and trailing space on EOF.
    df_edge = (
        "# escape=`\n"
        "FROM debian:12 AS base\n"
        "ENV A=1 `\n"
        "    # comment inside continuation\n"
        "    B=2\n"
        "FROM base AS app\n"
        "USER 1000\n"
        "RUN python3 -c \"x = a << EOF; y = '<<EOF'\" && "
        "echo $((a<<EOF)) && echo $(( 1 << shift )) && echo $((16<<shift)) && "
        "(( x = 1 << count )) && let a=16<<shift\n"
        "USER root:root\n"
        "RUN cat <<EOF   \n"
        "FROM fake_inside_heredoc\n"
        "EOF   \n"
    )
    patched = dockerfile_mod.patch_dockerfile_for_capsem_ca(df_edge).splitlines()
    # Base stage had no shell RUN, so it got CA_LINES without CA_UPDATE.
    base_idx = patched.index("FROM debian:12 AS base")
    assert patched[base_idx + 1 : base_idx + 4] == CA_LINES
    assert patched[base_idx + 4].startswith("ENV A=1")
    # Derived stage `FROM base AS app` has shell RUNs, so it gets CA_UPDATE right after FROM
    # and again after `USER root:root`!
    app_idx = patched.index("FROM base AS app")
    assert patched[app_idx + 1] == CA_UPDATE
    user_root_idx = patched.index("USER root:root")
    assert patched[user_root_idx + 1] == CA_UPDATE
    assert "FROM fake_inside_heredoc" in patched
    idx_fake = patched.index("FROM fake_inside_heredoc")
    assert patched[idx_fake + 1] == "EOF   "

    # Derived stage inheriting non-root USER from parent stage wraps CA_UPDATE in USER root / USER <prev>.
    df_inherited_nonroot = (
        "FROM ubuntu:24.04 AS parent\n"
        "ENV FOO=1\n"
        "USER appuser\n"
        "FROM parent AS child\n"
        "RUN curl -fsSL https://example.com\n"
    )
    patched_inh = dockerfile_mod.patch_dockerfile_for_capsem_ca(df_inherited_nonroot).splitlines()
    child_idx = patched_inh.index("FROM parent AS child")
    assert patched_inh[child_idx + 1 : child_idx + 4] == ["USER root", CA_UPDATE, "USER appuser"]

    # Stage switching to USER root followed only by exec-form RUN still emits CA_UPDATE after USER root.
    df_exec_after_root = (
        'FROM nonroot-base:latest\nUSER root\nRUN ["python3", "-c", "import urllib.request"]\n'
    )
    patched_exec = dockerfile_mod.patch_dockerfile_for_capsem_ca(df_exec_after_root).splitlines()
    uroot_idx = patched_exec.index("USER root")
    assert patched_exec[uroot_idx + 1] == CA_UPDATE

    with pytest.raises(ValueError, match="Unterminated heredoc <<EOF"):
        dockerfile_mod.patch_dockerfile_for_capsem_ca("FROM debian:12\nRUN cat <<EOF\nhello\n")

    # End-to-end CA fingerprint wiring in _build_dockerfile_in_vm invalidates cache on CA rotation.
    cache_dir = tmp_path / "ca-fp-cache"
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE_DIR", str(cache_dir))
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE", "1")
    df_ctx = tmp_path / "df_ctx"
    df_ctx.mkdir()
    df_file = df_ctx / "Dockerfile"
    df_file.write_text("FROM debian:12\nRUN echo hi\n")
    cfg_df = CapsemSandboxConfig(dockerfile=str(df_file)).to_container_spec()

    ctrl_fp1 = Scripted(
        [
            (dockerfile_mod._CA_READY, ok(f"{dockerfile_mod._CA_READY}:fp111\n")),
            (cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n")),
        ]
    )
    ctrl_fp1.files["/root/capsem-image-cache.tar.gz"] = b"archive-fp111"
    tag_fp1 = asyncio.run(runtime_mod._build_dockerfile_in_vm(ctrl_fp1, "vm-1", cfg_df))

    # Same CA fingerprint (fp111) in a new VM hits the cached archive without running `docker build`.
    ctrl_fp1_hit = Scripted([(dockerfile_mod._CA_READY, ok(f"{dockerfile_mod._CA_READY}:fp111\n"))])
    tag_fp1_hit = asyncio.run(runtime_mod._build_dockerfile_in_vm(ctrl_fp1_hit, "vm-2", cfg_df))
    assert tag_fp1_hit == tag_fp1
    assert not any(c.startswith("docker build") for c in ctrl_fp1_hit.commands)

    # Rotated CA fingerprint (fp222) misses the fp111 cache and builds a new image tag.
    ctrl_fp2 = Scripted(
        [
            (dockerfile_mod._CA_READY, ok(f"{dockerfile_mod._CA_READY}:fp222\n")),
            (cache_mod._IMAGE_SAVED, ok(f"{cache_mod._IMAGE_SAVED}\n")),
        ]
    )
    ctrl_fp2.files["/root/capsem-image-cache.tar.gz"] = b"archive-fp222"
    tag_fp2 = asyncio.run(runtime_mod._build_dockerfile_in_vm(ctrl_fp2, "vm-3", cfg_df))
    assert tag_fp2 != tag_fp1
    assert sum(c.startswith("docker build") for c in ctrl_fp2.commands) == 1


def test_dockerfile_workdir_extraction_and_resolution(tmp_path: Path) -> None:
    """When working_dir or user is unset, resolve effective WORKDIR and non-root USER from Dockerfile."""
    import inspect_capsem.containers.compose as c_compose_mod
    import inspect_capsem.sandbox as sb
    from inspect_capsem.sandbox import CapsemSandboxEnvironment

    def _defaults(text: str) -> dict[str, str]:
        p = tmp_path / "df_probe"
        p.write_text(text)
        return dockerfile_mod.resolve_dockerfile_defaults(p)

    assert _defaults("FROM debian:12\nRUN echo hi\n") == {}
    assert _defaults("FROM debian:12\nUSER developer\nUSER root\n") == {}
    assert _defaults("FROM debian:12 AS base\nUSER developer\nFROM base\n") == {"user": "developer"}
    assert _defaults("FROM ubuntu:24.04\nWORKDIR /app\nWORKDIR src\n") == {
        "working_dir": "/app/src"
    }
    assert _defaults("FROM ubuntu:24.04\nWORKDIR relative\n") == {"working_dir": "/relative"}
    assert _defaults(
        "ARG ROOT_DIR=/opt\n"
        "FROM ubuntu:24.04 AS base\n"
        "ARG ROOT_DIR\n"
        "ENV SUB_DIR=project\n"
        "ENV EXTRA sub/dir\n"
        'WORKDIR "${ROOT_DIR}/${SUB_DIR}"\n'
        "FROM base AS final\n"
        "WORKDIR ${EXTRA:-fallback}\n"
        "WORKDIR $UNSET_VAR\n"
    ) == {"working_dir": "/opt/project/sub/dir"}
    assert (
        _defaults(
            "FROM ubuntu:24.04 AS builder\n"
            "WORKDIR /build\n"
            "FROM debian:12\n"
            "COPY --from=builder /build /out\n"
        )
        == {}
    )

    # Direct Dockerfile path coercion (`vimgolf_challenges` pattern).
    df_vimgolf = tmp_path / "Dockerfile"
    df_vimgolf.write_text("FROM ubuntu:24.04\nUSER developer\nWORKDIR /app\n")
    coerced_str = compose_mod.coerce_config(str(df_vimgolf))
    assert (coerced_str.execution_mode, coerced_str.working_dir, coerced_str.user) == (
        "container",
        "/app",
        "developer",
    )

    # Explicit working_dir and user on CapsemSandboxConfig take precedence over Dockerfile.
    explicit_cfg = CapsemSandboxConfig(
        dockerfile=str(df_vimgolf), working_dir="/custom", user="customuser"
    )
    coerced_exp = compose_mod.coerce_config(explicit_cfg)
    assert (coerced_exp.working_dir, coerced_exp.user) == ("/custom", "customuser")

    # Compose service with build: and no working_dir (`inspect_harbor` pattern).
    rust_dir = tmp_path / "rust_task"
    rust_dir.mkdir()
    (rust_dir / "Dockerfile").write_text(
        "FROM rust:1.80\nUSER rustdev\nWORKDIR /app\nWORKDIR src\n"
    )
    harbor_compose_file = tmp_path / "harbor-compose.yaml"
    harbor_compose_file.write_text(f"services:\n  default:\n    build: {rust_dir}\n")
    extracted = c_compose_mod.extract_compose_fields(
        c_compose_mod.parse_compose_yaml_file(harbor_compose_file)
    )
    assert extracted["working_dir"] == "/app/src"
    assert extracted["user"] == "rustdev"

    ctrl = Scripted([("docker run -d", ok("cid-rust\n"))])
    original = sb.SdkCapsemController
    cast(Any, sb).SdkCapsemController = lambda: ctrl
    try:
        envs = asyncio.run(CapsemSandboxEnvironment.sample_init("t", str(harbor_compose_file), {}))
    finally:
        cast(Any, sb).SdkCapsemController = original
    env = cast(CapsemSandboxEnvironment, envs["default"])
    assert env._working_dir == "/app/src"
    assert env._user == "rustdev"
    run_cmd = next(c for c in ctrl.commands if c.startswith("docker run -d"))
    assert "-w /app/src" in run_cmd


def test_lower_dockerfile_heredocs_and_noca_staging(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Lower BuildKit RUN <<EOF and COPY <<EOF heredocs for classic docker build."""
    df_heredoc = (
        "FROM ubuntu:24.04\n"
        "RUN <<EOF\n"
        "apt-get update\n"
        'echo "hello $USER"\n'
        "EOF\n"
        "RUN <<'PYEOF'\n"
        "#!/usr/bin/env python3\n"
        "import sys\n"
        "print('ok')\n"
        "PYEOF\n"
        "RUN --mount=type=cache,target=/root/.cache cat <<EOF > /etc/greeting.txt\n"
        "line1\n"
        "line2\n"
        "EOF\n"
        "RUN cat <<-EOF1 > /tmp/a && cat <<'EOF2' > /tmp/b\n"
        "\talpha\n"
        "\tEOF1\n"
        "beta\n"
        "EOF2\n"
        "COPY --chmod=0755 --chown=1000:1000 <<'SCRIPT' /usr/local/bin/entry.sh\n"
        "#!/bin/sh\n"
        'exec "$@"\n'
        "SCRIPT\n"
        "COPY <<F1 <<F2 /etc/ multi/\n"
        "one\n"
        "F1\n"
        "two\n"
        "F2\n"
    )
    lowered = dockerfile_mod.lower_dockerfile_heredocs(df_heredoc)
    assert "<<EOF" not in lowered and "<<'PYEOF'" not in lowered and "<<'SCRIPT'" not in lowered
    lines = lowered.splitlines()
    assert any(
        ln.startswith("RUN _f=$(mktemp) && printf '%s\\n'") and 'sh "$_f"' in ln for ln in lines
    )
    assert any(
        ln.startswith("RUN _f=$(mktemp) && printf '%s\\n'") and '/usr/bin/env python3 "$_f"' in ln
        for ln in lines
    )
    assert any("| { cat > /etc/greeting.txt; }" in ln for ln in lines)
    assert any("/tmp/.capsem_hd_0" in ln and "/tmp/.capsem_hd_1" in ln for ln in lines)
    assert any(
        "mkdir -p /usr/local/bin" in ln
        and "> /usr/local/bin/entry.sh" in ln
        and "chmod 0755 /usr/local/bin/entry.sh" in ln
        and "chown 1000:1000 /usr/local/bin/entry.sh" in ln
        for ln in lines
    )
    assert any("multi/F1" in ln and "multi/F2" in ln for ln in lines)

    # Execute lowered bare RUN <<EOF under /bin/sh: variables, loops, and non-errexit behavior.
    bare_df = (
        "FROM alpine:3.20\n"
        "RUN <<EOF\n"
        "X=hello\n"
        'echo "X=$X"\n'
        "for p in a b; do echo p=$p; done\n"
        "false\n"
        "echo still-ran\n"
        "EOF\n"
    )
    bare_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(bare_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_bare = subprocess.run(
        ["/bin/sh", "-c", bare_cmd], capture_output=True, text=True, check=False
    )
    assert proc_bare.returncode == 0
    assert proc_bare.stdout == "X=hello\np=a\np=b\nstill-ran\n"

    # Non-zero final exit status propagates out of bare RUN <<EOF.
    fail_df = "FROM alpine:3.20\nRUN <<EOF\necho before-fail\nexit 7\nEOF\n"
    fail_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(fail_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_fail = subprocess.run(
        ["/bin/sh", "-c", fail_cmd], capture_output=True, text=True, check=False
    )
    assert proc_fail.returncode == 7
    assert proc_fail.stdout == "before-fail\n"

    # Shebang script executes with its interpreter and preserves script variables.
    shebang_df = (
        "FROM alpine:3.20\n"
        "RUN <<EOF\n"
        "#!/usr/bin/env python3\n"
        "vals = [f'v={i}' for i in range(2)]\n"
        "print(' '.join(vals))\n"
        "EOF\n"
    )
    shebang_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(shebang_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_py = subprocess.run(
        ["/bin/sh", "-c", shebang_cmd], capture_output=True, text=True, check=False
    )
    assert proc_py.returncode == 0
    assert proc_py.stdout == "v=0 v=1\n"

    # Unquoted `cmd <<EOF` preserves \\, \$, \` while escaping " and lone backslashes.
    cmd_hd_df = 'FROM alpine:3.20\nRUN cat <<EOF\na\\\\b \\$X "$Y" \\z\nEOF\n'
    cmd_hd = (
        dockerfile_mod.lower_dockerfile_heredocs(cmd_hd_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_cmd = subprocess.run(
        ["/bin/sh", "-c", cmd_hd],
        env={**os.environ, "X": "ignored", "Y": "expanded"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc_cmd.returncode == 0
    assert proc_cmd.stdout == 'a\\b $X "expanded" \\z\n'

    # Unquoted COPY <<EOF expands $VAR/${VAR} from env without running $(...) or `...`.
    copy_dest = tmp_path / "copied.txt"
    copy_df = (
        f"FROM alpine:3.20\nCOPY <<EOF {copy_dest}\nhello $GREET $(echo pwned) `echo pwn2`\nEOF\n"
    )
    copy_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(copy_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_copy = subprocess.run(
        ["/bin/sh", "-c", copy_cmd],
        env={**os.environ, "GREET": "world"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc_copy.returncode == 0
    assert copy_dest.read_text() == "hello world $(echo pwned) `echo pwn2`\n"

    # Empty heredoc body and unterminated heredoc error.
    empty_hd = dockerfile_mod.lower_dockerfile_heredocs("FROM alpine:3.20\nRUN <<EOF\nEOF\n")
    assert "printf ''" in empty_hd
    with pytest.raises(ValueError, match="Unterminated heredoc <<EOF"):
        dockerfile_mod.lower_dockerfile_heredocs("FROM alpine:3.20\nRUN <<EOF\nunterminated\n")

    # No-CA VM build still stages the lowered Dockerfile when heredocs were lowered.
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE", "0")
    df_path = tmp_path / "Dockerfile"
    df_path.write_text("FROM alpine:3.20\nCOPY <<EOF /app/hello.txt\nworld\nEOF\n")
    ctrl_noca = Scripted([(dockerfile_mod._CA_READY, ok("NO_CAPSEM_CA\n"))])
    asyncio.run(
        runtime_mod._build_dockerfile_in_vm(
            ctrl_noca,
            "vm-noca",
            CapsemSandboxConfig(dockerfile=str(df_path)).to_container_spec(),
        )
    )
    assert f"{dockerfile_mod._CA_DF_DIR}.tar.gz" in ctrl_noca.uploads
    build_cmd = next(c for c in ctrl_noca.commands if c.startswith("docker build"))
    assert f"-f {dockerfile_mod._CA_DF_DIR}/Dockerfile" in build_cmd
