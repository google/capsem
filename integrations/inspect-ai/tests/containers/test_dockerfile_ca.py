"""Pierre's original pure CA transformation regressions."""

import inspect_capsem.containers.dockerfile as dockerfile_mod
import pytest

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
    n_ca = len(CA_LINES)
    assert patched[2 : 2 + n_ca + 1] == [*CA_LINES, CA_UPDATE]
    idx = patched.index("    python:3.12 AS tools")
    # Exec-form RUN only: the base may have no shell, so no update RUN.
    assert patched[idx + 1 : idx + 1 + n_ca + 1] == [*CA_LINES, 'RUN ["pip", "install", "x"]']
    # Stages built from an earlier stage inherit the CA; scratch gets nothing.
    assert patched[-3:] == ["FROM build AS test", "FROM scratch", "COPY --from=build /out /out"]
    assert sum(line == CA_LINES[0] for line in patched) == 2
    assert sum(line == CA_UPDATE for line in patched) == 1


def test_ca_patch_keeps_image_bundle() -> None:
    patched = dockerfile_mod.patch_dockerfile_for_capsem_ca("FROM alpine:3.20\nRUN apk add curl\n")
    assert patched.splitlines() == ["FROM alpine:3.20", *CA_LINES, CA_UPDATE, "RUN apk add curl"]
    assert "/etc/ssl/certs/ca-certificates.crt" not in patched
    assert "COPY .capsem-ca-bundle.crt /usr/local/share/capsem/ca-bundle.crt" in patched
    assert "SSL_CERT_FILE=/usr/local/share/capsem/ca-bundle.crt" in patched
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


def test_ca_patcher_edge_cases_escape_quoted_heredoc_and_derived_stage() -> None:
    df_edge = (
        "# escape=`\nFROM debian:12 AS base\nENV A=1 `\n"
        "    # comment inside continuation\n    B=2\nFROM base AS app\nUSER 1000\n"
        "RUN python3 -c \"x = a << EOF; y = '<<EOF'\" && echo $((a<<EOF)) "
        "&& echo $(( 1 << shift )) && echo $((16<<shift)) "
        "&& (( x = 1 << count )) && let a=16<<shift\n"
        "USER root:root\nRUN cat <<EOF   \nFROM fake_inside_heredoc\nEOF   \n"
    )
    patched = dockerfile_mod.patch_dockerfile_for_capsem_ca(df_edge).splitlines()
    base_idx = patched.index("FROM debian:12 AS base")
    n_ca = len(CA_LINES)
    assert patched[base_idx + 1 : base_idx + 1 + n_ca] == CA_LINES
    assert patched[base_idx + 1 + n_ca].startswith("ENV A=1")
    app_idx = patched.index("FROM base AS app")
    assert patched[app_idx + 1] == CA_UPDATE
    user_root_idx = patched.index("USER root:root")
    assert patched[user_root_idx + 1] == CA_UPDATE
    assert "FROM fake_inside_heredoc" in patched
    idx_fake = patched.index("FROM fake_inside_heredoc")
    assert patched[idx_fake + 1] == "EOF   "
    df_inherited_nonroot = (
        "FROM ubuntu:24.04 AS parent\nENV FOO=1\nUSER appuser\n"
        "FROM parent AS child\nRUN curl -fsSL https://example.com\n"
    )
    patched_inh = dockerfile_mod.patch_dockerfile_for_capsem_ca(df_inherited_nonroot).splitlines()
    child_idx = patched_inh.index("FROM parent AS child")
    assert patched_inh[child_idx + 1 : child_idx + 4] == ["USER root", CA_UPDATE, "USER appuser"]
    df_exec_after_root = (
        'FROM nonroot-base:latest\nUSER root\nRUN ["python3", "-c", "import urllib.request"]\n'
    )
    patched_exec = dockerfile_mod.patch_dockerfile_for_capsem_ca(df_exec_after_root).splitlines()
    uroot_idx = patched_exec.index("USER root")
    assert patched_exec[uroot_idx + 1] == CA_UPDATE
    with pytest.raises(ValueError, match="Unterminated heredoc <<EOF"):
        dockerfile_mod.patch_dockerfile_for_capsem_ca("FROM debian:12\nRUN cat <<EOF\nhello\n")
