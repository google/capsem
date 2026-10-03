"""Compose YAML parsing, interpolation, and field extraction tests."""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import inspect_capsem._compose as compose_mod
import inspect_capsem.containers.compose as c_compose_mod
import inspect_capsem.containers.runtime as runtime_mod
import pytest
from inspect_capsem import CapsemSandboxConfig

from ..conftest import Scripted, ok


def test_compose_field_extraction(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    extract = c_compose_mod.extract_compose_fields
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        extract({"services": {}})
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        extract({"execution_mode": "vm"})
    ctx = tmp_path / "ctx"
    ctx.mkdir()
    with pytest.raises(ValueError, match="Multi-service Compose files are not supported"):
        extract(
            {
                "services": {
                    "a": {"image": "img:a"},
                    "b": {"image": "img:b"},
                }
            },
            base_dir=tmp_path,
        )
    services = {
        "b": {"image": None, "build": "ctx", "working_dir": "/w"},
    }
    fields = extract({"services": services}, base_dir=tmp_path)
    assert fields == {
        "execution_mode": "container",
        "dockerfile": str(ctx / "Dockerfile"),
        "working_dir": "/w",
    }
    services = {"default": {"build": "Dockerfile.x"}}
    assert extract({"services": services})["dockerfile"] == "Dockerfile.x"
    build = {"context": "sub", "dockerfile": "nested/Custom"}
    services = {"svc": {"build": build}}
    sub_fields = extract({"services": services}, base_dir=tmp_path)
    assert sub_fields["dockerfile"] == str(tmp_path / "sub" / "nested" / "Custom")
    assert sub_fields["build_context"] == str(tmp_path / "sub")
    services = {"svc": {"build": {"context": "."}}}
    assert extract({"services": services})["dockerfile"] == "Dockerfile"

    (tmp_path / "named").mkdir()
    monkeypatch.setenv("INHERITED_VAR", "from_os")
    rich_compose = {
        "services": {
            "app": {
                "image": "ubuntu:24.04",
                "environment": ["K=V", "INHERITED_VAR"],
                "command": "echo hi",
                "entrypoint": "/ep",
                "volumes": ["./ctx:/mnt", "/abs:/abs", "named:/data"],
                "ports": [8080],
                "expose": [9000],
                "init": True,
                "deploy": {"resources": {"limits": {"memory": "256m"}}},
                "network_mode": "host",
                "user": "nobody",
                "healthcheck": {"test": ("CMD", "true"), "retries": 5},
            }
        }
    }
    rich = extract(rich_compose, base_dir=tmp_path)
    assert rich["environment"] == {"K": "V", "INHERITED_VAR": "from_os"}
    assert rich["command"] == "echo hi"
    assert rich["entrypoint"] == "/ep"
    assert rich["volumes"] == (f"{ctx.resolve()}:/mnt", "/abs:/abs", "named:/data")
    assert rich["ports"] == ("8080",)
    assert rich["expose"] == ("9000",)
    assert rich["init"] is True
    assert rich["mem_limit"] == "256m"
    assert rich["network_mode"] == "host"
    assert rich["user"] == "nobody"
    assert rich["healthcheck"] == {"test": ["CMD", "true"], "retries": 5}


def test_compose_networks_internal_and_network_modes() -> None:
    nets = {"isolated": {"internal": True}, "ext": {"internal": False}}
    assert (
        c_compose_mod.extract_compose_fields(
            {
                "networks": nets,
                "services": {"offline_app": {"image": "alpine:3.20", "networks": ["isolated"]}},
            }
        )["network_mode"]
        == "none"
    )
    assert "network_mode" not in c_compose_mod.extract_compose_fields(
        {
            "networks": nets,
            "services": {"online_app": {"image": "alpine:3.20", "networks": ["isolated", "ext"]}},
        }
    )
    assert (
        c_compose_mod.extract_compose_fields(
            {
                "networks": nets,
                "services": {
                    "dict_offline": {
                        "image": "alpine:3.20",
                        "networks": {"isolated": {}, "local_only": {"internal": True}},
                    }
                },
            }
        )["network_mode"]
        == "none"
    )

    default_internal = {
        "networks": {"default": {"internal": True}},
        "services": {"app": {"image": "alpine:3.20"}},
    }
    assert c_compose_mod.extract_compose_fields(default_internal)["network_mode"] == "none"

    # --network none, bridge/default map to host, and service:<name> raises ValueError.
    cmd_none = runtime_mod._build_docker_run_command(
        "c-none", "alpine:3.20", CapsemSandboxConfig(network_mode="none").to_container_spec(), []
    )
    assert "--network none" in cmd_none
    for mode in ("bridge", "default"):
        cmd_br = runtime_mod._build_docker_run_command(
            "c-br", "alpine:3.20", CapsemSandboxConfig(network_mode=mode).to_container_spec(), []
        )
        assert "--network host" in cmd_br
    with pytest.raises(ValueError, match="service:db"):
        runtime_mod._build_docker_run_command(
            "c-svc",
            "alpine:3.20",
            CapsemSandboxConfig(network_mode="service:db").to_container_spec(),
            [],
        )


def test_compose_semantics_and_container_launch_alignment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    """Verify Compose interpolation, long-form specs, build args/target, depends_on, and launch flags."""
    # 1. Compose variable interpolation (.env + os.environ precedence + ${VAR:-/:/?} + nested + bare $VAR + $$).
    (tmp_path / ".env").write_text(
        "# comment\n"
        "export DOTENV_ONLY='from_dotenv'\n"
        'BOTH_SET="from_dotenv_overridden"\n'
        "INLINE_CMT=bar_val # inline comment stripped\n"
        'QUOTED_DBL_CMT="x"  # c\n'
        "QUOTED_SGL_CMT='q' # c\n"
        'QUOTED_NOSPACE_CMT="x"#c\n'
        'QUOTED_TRAILING="x" trailing\n'
        'QUOTED_DOT_TAIL="x" a.b\n'
        "QUOTED_SGL_TRAILING='a'b\n"
        'QUOTED_ESCQ_CMT="esc \\" q" # c\n'
        'QUOTED_ESCQ_ONLY="esc \\" q"\n'
        'QUOTED_ESC_N="a\\nb"\n'
        'QUOTED_ESC_DOLLAR="a\\$b"\n'
        "QUOTED_SGL_ESC='a\\nb'\n"
        "DOT_A=a1 # comment\n"
        'DOT_B="b # keep"\n'
        "DOT_C=#onlycomment\n"
        "DOT_D=x#y\n"
        "DOT_E= #c\n"
        "DOT_F=v\t#tab\n"
        "SPACES_CMT=  v  # c\n"
        "BARE_HASH=v #\n"
        'DERIVED="${DOTENV_ONLY}/sub"\n'
        "EMPTY_VAR=\n"
    )
    monkeypatch.setenv("BOTH_SET", "from_os_env")
    monkeypatch.setenv("EMPTY_VAR", "")
    monkeypatch.setenv("W", "wv")
    monkeypatch.delenv("UNSET_VAR", raising=False)
    monkeypatch.delenv("U", raising=False)
    monkeypatch.delenv("V", raising=False)

    compose_interp = tmp_path / "compose-interp.yaml"
    compose_interp.write_text(
        "services:\n"
        "  app:\n"
        "    image: myrepo/${DOTENV_ONLY}:${BOTH_SET}\n"
        "    environment:\n"
        "      DEF_COLON: ${EMPTY_VAR:-colon_fallback}\n"
        "      DEF_PLAIN_EMPTY: ${EMPTY_VAR-plain_fallback}\n"
        "      DEF_PLAIN_UNSET: ${UNSET_VAR-unset_fallback}\n"
        "      ALT_COLON_SET: ${W:+alt_w}\n"
        "      ALT_COLON_EMPTY: ${EMPTY_VAR:+alt_empty}\n"
        "      ALT_COLON_UNSET: ${U:+alt_u}\n"
        "      ALT_PLAIN_SET: ${W+alt_plain_w}\n"
        "      ALT_PLAIN_EMPTY: ${EMPTY_VAR+alt_plain_empty}\n"
        "      ALT_PLAIN_UNSET: ${U+alt_plain_u}\n"
        "      NESTED_DEF: ${U:-${V:-inner}}\n"
        "      BARE_VAR: $W\n"
        "      ESCAPED_BARE: $$W\n"
        "      ESCAPED: $$LITERAL_DOLLAR\n"
        "      FROM_INLINE: ${INLINE_CMT}\n"
        "      FROM_QUOTED_DBL_CMT: ${QUOTED_DBL_CMT}\n"
        "      FROM_QUOTED_SGL_CMT: ${QUOTED_SGL_CMT}\n"
        "      FROM_QUOTED_NOSPACE_CMT: ${QUOTED_NOSPACE_CMT}\n"
        "      FROM_QUOTED_TRAILING: ${QUOTED_TRAILING}\n"
        "      FROM_QUOTED_DOT_TAIL: ${QUOTED_DOT_TAIL}\n"
        "      FROM_QUOTED_SGL_TRAILING: ${QUOTED_SGL_TRAILING}\n"
        "      FROM_QUOTED_ESCQ_CMT: ${QUOTED_ESCQ_CMT}\n"
        "      FROM_QUOTED_ESCQ_ONLY: ${QUOTED_ESCQ_ONLY}\n"
        "      FROM_QUOTED_ESC_N: ${QUOTED_ESC_N}\n"
        "      FROM_QUOTED_ESC_DOLLAR: ${QUOTED_ESC_DOLLAR}\n"
        "      FROM_QUOTED_SGL_ESC: ${QUOTED_SGL_ESC}\n"
        "      FROM_DOT_A: ${DOT_A}\n"
        "      FROM_DOT_B: ${DOT_B}\n"
        "      FROM_DOT_C: ${DOT_C}\n"
        "      FROM_DOT_D: ${DOT_D}\n"
        "      FROM_DOT_E: ${DOT_E}\n"
        "      FROM_DOT_F: ${DOT_F}\n"
        "      FROM_SPACES_CMT: ${SPACES_CMT}\n"
        "      FROM_BARE_HASH: ${BARE_HASH}\n"
        "      FROM_DERIVED: ${DERIVED}\n"
    )
    parsed_interp = c_compose_mod.parse_compose_yaml_file(compose_interp)
    svc_app = parsed_interp["services"]["app"]
    assert svc_app["image"] == "myrepo/from_dotenv:from_os_env"
    assert svc_app["environment"] == {
        "DEF_COLON": "colon_fallback",
        "DEF_PLAIN_EMPTY": "",
        "DEF_PLAIN_UNSET": "unset_fallback",
        "ALT_COLON_SET": "alt_w",
        "ALT_COLON_EMPTY": "",
        "ALT_COLON_UNSET": "",
        "ALT_PLAIN_SET": "alt_plain_w",
        "ALT_PLAIN_EMPTY": "alt_plain_empty",
        "ALT_PLAIN_UNSET": "",
        "NESTED_DEF": "inner",
        "BARE_VAR": "wv",
        "ESCAPED_BARE": "$W",
        "ESCAPED": "$LITERAL_DOLLAR",
        "FROM_INLINE": "bar_val",
        "FROM_QUOTED_DBL_CMT": "x",
        "FROM_QUOTED_SGL_CMT": "q",
        "FROM_QUOTED_NOSPACE_CMT": "x",
        "FROM_QUOTED_TRAILING": "x",
        "FROM_QUOTED_DOT_TAIL": "x",
        "FROM_QUOTED_SGL_TRAILING": "a",
        "FROM_QUOTED_ESCQ_CMT": 'esc " q',
        "FROM_QUOTED_ESCQ_ONLY": 'esc " q',
        "FROM_QUOTED_ESC_N": "a\nb",
        "FROM_QUOTED_ESC_DOLLAR": "a$b",
        "FROM_QUOTED_SGL_ESC": "a\\nb",
        "FROM_DOT_A": "a1",
        "FROM_DOT_B": "b # keep",
        "FROM_DOT_C": "#onlycomment",
        "FROM_DOT_D": "x#y",
        "FROM_DOT_E": "#c",
        "FROM_DOT_F": "v\t#tab",
        "FROM_SPACES_CMT": "v",
        "FROM_BARE_HASH": "v",
        "FROM_DERIVED": "from_dotenv/sub",
    }

    for bad_dotenv in ('K="multi\n', 'K="multi\\\n', "K='multi\n"):
        unterm_path = tmp_path / ".env.unterm"
        unterm_path.write_text(bad_dotenv)
        with pytest.raises(ValueError, match=r"Unterminated .*quoted value"):
            c_compose_mod._load_dotenv(unterm_path)

    for bad_tail in (
        'K="x" "y"\n',
        "K='x' 'y'\n",
        'K="x" OTHER=1\n',
        'K="x" foo bar\n',
        'K="x" w # c\n',
    ):
        tail_path = tmp_path / ".env.tail"
        tail_path.write_text(bad_tail)
        with pytest.raises(ValueError, match="Unexpected trailing characters"):
            c_compose_mod._load_dotenv(tail_path)

    compose_err_colon = tmp_path / "compose-err-colon.yaml"
    compose_err_colon.write_text("services:\n  app:\n    image: ${EMPTY_VAR:?must not be empty}\n")
    with pytest.raises(ValueError, match="must not be empty"):
        c_compose_mod.parse_compose_yaml_file(compose_err_colon)

    compose_err_plain = tmp_path / "compose-err-plain.yaml"
    compose_err_plain.write_text("services:\n  app:\n    image: ${UNSET_VAR?must be set}\n")
    with pytest.raises(ValueError, match="must be set"):
        c_compose_mod.parse_compose_yaml_file(compose_err_plain)

    # ${EMPTY_VAR?err} succeeds with "" when variable is set but empty.
    compose_empty_ok = tmp_path / "compose-empty-ok.yaml"
    compose_empty_ok.write_text("services:\n  app:\n    image: alpine:3.20${EMPTY_VAR?err}\n")
    assert (
        c_compose_mod.parse_compose_yaml_file(compose_empty_ok)["services"]["app"]["image"]
        == "alpine:3.20"
    )

    # Unclosed ${U and malformed ${A B} raise ValueError.
    for bad_expr in ("${U", "prefix-${U-unclosed", "${A B}", "${}"):
        bad_compose = tmp_path / "compose-bad-interp.yaml"
        bad_compose.write_text(f"services:\n  app:\n    image: {bad_expr}\n")
        with pytest.raises(ValueError, match="Invalid Compose variable interpolation"):
            c_compose_mod.parse_compose_yaml_file(bad_compose)

    # 2. Long-form volumes, ports, expose, and dict environment (None/bool).
    bind_dir = tmp_path / "src"
    bind_dir.mkdir()
    monkeypatch.setenv("INHERITED_DICT_KEY", "inherited_val")
    monkeypatch.delenv("UNSET_DICT_KEY", raising=False)
    long_compose = {
        "services": {
            "web": {
                "image": "nginx:alpine",
                "environment": {
                    "INHERITED_DICT_KEY": None,
                    "UNSET_DICT_KEY": None,
                    "BOOL_T": True,
                    "BOOL_F": False,
                },
                "volumes": [
                    {
                        "type": "bind",
                        "source": "./src",
                        "target": "/app/src",
                        "read_only": True,
                        "consistency": "cached",
                    },
                    {"type": "volume", "source": "named-vol", "target": "/var/lib/data"},
                ],
                "ports": [
                    {"target": 80, "published": 8080, "protocol": "tcp"},
                    {"target": 53, "published": "5353", "host_ip": "127.0.0.1", "protocol": "udp"},
                    {"target": 9000},
                ],
                "expose": [{"target": 3000, "protocol": "tcp"}],
            }
        }
    }
    with caplog.at_level(logging.WARNING):
        ext_long = c_compose_mod.extract_compose_fields(long_compose, base_dir=tmp_path)
    assert "Ignoring unsupported Compose volume option(s) ['consistency']" in caplog.text
    assert ext_long["environment"] == {
        "INHERITED_DICT_KEY": "inherited_val",
        "UNSET_DICT_KEY": "",
        "BOOL_T": "true",
        "BOOL_F": "false",
    }
    assert ext_long["volumes"] == (f"{bind_dir.resolve()}:/app/src:ro", "named-vol:/var/lib/data")
    assert ext_long["ports"] == ("8080:80", "127.0.0.1:5353:53/udp", "9000")
    assert ext_long["expose"] == ("3000",)

    with pytest.raises(ValueError, match="volume type 'tmpfs'"):
        c_compose_mod._normalize_volumes([{"type": "tmpfs", "target": "/tmp"}], tmp_path)
    with pytest.raises(ValueError, match="requires both 'source' and 'target'"):
        c_compose_mod._normalize_volumes([{"type": "bind", "source": "./src"}], tmp_path)
    with pytest.raises(ValueError, match="requires 'target'"):
        c_compose_mod._normalize_ports([{"published": 8080}], field_name="ports")

    # 3. build.args, build.target, build.dockerfile_inline, and unsupported build options.
    monkeypatch.setenv("INSPECT_CAPSEM_IMAGE_CACHE", "0")
    df_file = tmp_path / "Dockerfile"
    df_file.write_text("FROM alpine:3.20 AS builder\nARG FOO=default\nRUN echo $FOO\n")
    build_compose = {
        "services": {
            "app": {
                "build": {
                    "context": ".",
                    "args": {"FOO": "custom", "DEBUG": True},
                    "target": "builder",
                }
            }
        }
    }
    ext_build = c_compose_mod.extract_compose_fields(build_compose, base_dir=tmp_path)
    assert ext_build["build_args"] == {"FOO": "custom", "DEBUG": "true"}
    assert ext_build["build_target"] == "builder"

    ctrl_build = Scripted()
    cfg_b1 = CapsemSandboxConfig(**ext_build)
    tag1 = asyncio.run(
        runtime_mod._build_dockerfile_in_vm(ctrl_build, "vm-b", cfg_b1.to_container_spec())
    )
    docker_build_cmd = next(c for c in ctrl_build.commands if c.startswith("docker build"))
    assert "--target builder" in docker_build_cmd
    assert "--build-arg DEBUG=true" in docker_build_cmd
    assert "--build-arg FOO=custom" in docker_build_cmd

    # Changing build_args changes the image tag digest.
    cfg_b2 = cfg_b1.model_copy(update={"build_args": {"FOO": "other", "DEBUG": "true"}})
    tag2 = asyncio.run(
        runtime_mod._build_dockerfile_in_vm(ctrl_build, "vm-b", cfg_b2.to_container_spec())
    )
    assert tag2 != tag1

    with pytest.raises(ValueError, match="dockerfile_inline"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"build": {"dockerfile_inline": "FROM alpine\n"}}}},
            base_dir=tmp_path,
        )
    for bad_build_key in ("ssh", "secrets", "cache_from", "network", "platforms", "extra_hosts"):
        with pytest.raises(ValueError, match="Unsupported Compose build option"):
            c_compose_mod.extract_compose_fields(
                {"services": {"app": {"build": {"context": ".", bad_build_key: ["x"]}}}},
                base_dir=tmp_path,
            )

    # 4. Multi-service Compose files, depends_on, and env_file fail fast with ValueError;
    #    unknown service keys log WARNING while x-* extension keys do not warn.
    with pytest.raises(ValueError, match="Multi-service Compose files are not supported"):
        c_compose_mod.extract_compose_fields(
            {
                "services": {
                    "app": {"image": "app:1"},
                    "db": {"image": "postgres:16"},
                }
            }
        )
    with pytest.raises(ValueError, match="depends_on"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"image": "alpine", "depends_on": ["db"]}}}
        )
    with pytest.raises(ValueError, match="env_file"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"image": "alpine", "env_file": [".env"]}}}
        )
    caplog.clear()
    with caplog.at_level(logging.WARNING):
        c_compose_mod.extract_compose_fields(
            {
                "services": {
                    "app": {
                        "image": "alpine",
                        "shm_size": "64m",
                        "tmpfs": ["/run"],
                        "cap_add": ["NET_ADMIN"],
                        "privileged": True,
                        "cpus": "2",
                        "platform": "linux/amd64",
                        "x-inspect": {"k": "v"},
                    }
                }
            }
        )
    assert (
        "Ignoring unsupported compose keys in service 'app': "
        "cap_add, cpus, platform, privileged, shm_size, tmpfs" in caplog.text
    )
    assert "x-inspect" not in caplog.text

    # 5 & 8. String entrypoint/command tokenization + omitting -w when working_dir is unset.
    cmd_ep = runtime_mod._build_docker_run_command(
        "c-ep",
        "img:1",
        CapsemSandboxConfig(
            entrypoint="/usr/bin/env VAR=1", command="python3 -c 'print(1, 2)'"
        ).to_container_spec(),
        [],
    )
    assert "--entrypoint /usr/bin/env" in cmd_ep
    assert "VAR=1 python3 -c 'print(1, 2)'" in cmd_ep
    assert "sh -c" not in cmd_ep
    assert " -w " not in cmd_ep

    cmd_empty_ep = runtime_mod._build_docker_run_command(
        "c-empty-ep",
        "img:1",
        CapsemSandboxConfig(entrypoint="", working_dir="/workspace").to_container_spec(),
        [],
    )
    assert '--entrypoint ""' in cmd_empty_ep
    assert "-w /workspace" in cmd_empty_ep

    # 6 & 7. resolve_compose_file + start_container_for_init, and explicit model_fields_set wins.
    compose_file = tmp_path / "compose-run.yaml"
    compose_file.write_text(
        "services:\n  web:\n    image: custom-web:2.0\n    working_dir: /srv/web\n"
    )
    ctrl_init = Scripted([("docker run -d", ok("cid-web\n"))])
    asyncio.run(
        runtime_mod.start_container_for_init(
            ctrl_init,
            "vm-1",
            compose_mod.resolve_compose_file(
                CapsemSandboxConfig(compose_file=str(compose_file))
            ).to_container_spec(),
        )
    )
    run_web = next(c for c in ctrl_init.commands if c.startswith("docker run -d"))
    assert "custom-web:2.0" in run_web and "-w /srv/web" in run_web

    # Explicit default-valued fields on CapsemSandboxConfig win over compose_file values.
    explicit_default_cfg = CapsemSandboxConfig(
        compose_file=str(compose_file), image="python:3.11-slim", working_dir="/workspace"
    )
    resolved_explicit = compose_mod.resolve_compose_file(explicit_default_cfg)
    assert resolved_explicit.image == "python:3.11-slim"
    assert resolved_explicit.working_dir == "/workspace"

    # 9. coerce_config treats arbitrary OCI image:tag strings as images.
    for img_ref in (
        "redis:7",
        "postgres:16-alpine",
        "nvidia/cuda:12.4-base",
        "localhost:5000/my-img:1.0",
        "my-custom-img:v1",
    ):
        coerced_img = compose_mod.coerce_config(img_ref)
        assert coerced_img.image == img_ref
