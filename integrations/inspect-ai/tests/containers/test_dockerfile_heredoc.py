"""Original pure lowering and real shell/interpreter semantics."""

import os
import subprocess
from pathlib import Path

import inspect_capsem.containers.dockerfile as dockerfile_mod
import pytest


def test_lower_dockerfile_heredocs(tmp_path: Path) -> None:
    """Lower BuildKit RUN <<EOF and COPY <<EOF heredocs for classic docker build."""
    df_heredoc = "FROM ubuntu:24.04\nRUN <<EOF\napt-get update\necho \"hello $USER\"\nEOF\nRUN <<'PYEOF'\n#!/usr/bin/env python3\nimport sys\nprint('ok')\nPYEOF\nRUN --mount=type=cache,target=/root/.cache cat <<EOF > /etc/greeting.txt\nline1\nline2\nEOF\nRUN cat <<-EOF1 > /tmp/a && cat <<'EOF2' > /tmp/b\n\talpha\n\tEOF1\nbeta\nEOF2\nCOPY --chmod=0755 --chown=1000:1000 <<'SCRIPT' /usr/local/bin/entry.sh\n#!/bin/sh\nexec \"$@\"\nSCRIPT\nCOPY <<F1 <<F2 /etc/ multi/\none\nF1\ntwo\nF2\n"
    lowered = dockerfile_mod.lower_dockerfile_heredocs(df_heredoc)
    assert "<<EOF" not in lowered and "<<'PYEOF'" not in lowered and ("<<'SCRIPT'" not in lowered)
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
        and ("chmod 0755 /usr/local/bin/entry.sh" in ln)
        and ("chown 1000:1000 /usr/local/bin/entry.sh" in ln)
        for ln in lines
    )
    assert any("multi/F1" in ln and "multi/F2" in ln for ln in lines)
    bare_df = 'FROM alpine:3.20\nRUN <<EOF\nX=hello\necho "X=$X"\nfor p in a b; do echo p=$p; done\nfalse\necho still-ran\nEOF\n'
    bare_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(bare_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_bare = subprocess.run(
        ["/bin/sh", "-c", bare_cmd],
        capture_output=True,
        text=True,
        check=False,
        timeout=5,
        env={"PATH": os.defpath},
    )
    assert proc_bare.returncode == 0
    assert proc_bare.stdout == "X=hello\np=a\np=b\nstill-ran\n"
    fail_df = "FROM alpine:3.20\nRUN <<EOF\necho before-fail\nexit 7\nEOF\n"
    fail_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(fail_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_fail = subprocess.run(
        ["/bin/sh", "-c", fail_cmd],
        capture_output=True,
        text=True,
        check=False,
        timeout=5,
        env={"PATH": os.defpath},
    )
    assert proc_fail.returncode == 7
    assert proc_fail.stdout == "before-fail\n"
    shebang_df = "FROM alpine:3.20\nRUN <<EOF\n#!/usr/bin/env python3\nvals = [f'v={i}' for i in range(2)]\nprint(' '.join(vals))\nEOF\n"
    shebang_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(shebang_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_py = subprocess.run(
        ["/bin/sh", "-c", shebang_cmd],
        capture_output=True,
        text=True,
        check=False,
        timeout=5,
        env={"PATH": os.defpath},
    )
    assert proc_py.returncode == 0
    assert proc_py.stdout == "v=0 v=1\n"
    cmd_hd_df = 'FROM alpine:3.20\nRUN cat <<EOF\na\\\\b \\$X "$Y" \\z\nEOF\n'
    cmd_hd = (
        dockerfile_mod.lower_dockerfile_heredocs(cmd_hd_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_cmd = subprocess.run(
        ["/bin/sh", "-c", cmd_hd],
        env={"PATH": os.defpath, "X": "ignored", "Y": "expanded"},
        capture_output=True,
        text=True,
        check=False,
        timeout=5,
    )
    assert proc_cmd.returncode == 0
    assert proc_cmd.stdout == 'a\\b $X "expanded" \\z\n'
    copy_dest = tmp_path / "copied.txt"
    copy_df = (
        f"FROM alpine:3.20\nCOPY <<EOF {copy_dest}\nhello $GREET $(echo pwned) `echo pwn2`\nEOF\n"
    )
    copy_cmd = (
        dockerfile_mod.lower_dockerfile_heredocs(copy_df).splitlines()[1].removeprefix("RUN ")
    )
    proc_copy = subprocess.run(
        ["/bin/sh", "-c", copy_cmd],
        env={"PATH": os.defpath, "GREET": "world"},
        capture_output=True,
        text=True,
        check=False,
        timeout=5,
    )
    assert proc_copy.returncode == 0
    assert copy_dest.read_text() == "hello world $(echo pwned) `echo pwn2`\n"
    empty_hd = dockerfile_mod.lower_dockerfile_heredocs("FROM alpine:3.20\nRUN <<EOF\nEOF\n")
    assert "printf ''" in empty_hd
    with pytest.raises(ValueError, match="Unterminated heredoc <<EOF"):
        dockerfile_mod.lower_dockerfile_heredocs("FROM alpine:3.20\nRUN <<EOF\nunterminated\n")
