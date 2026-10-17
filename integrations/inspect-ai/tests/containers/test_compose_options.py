"""Pure cases carried from Pierre Tholoniat's original bb61fc82d tests."""

import logging
from pathlib import Path

import inspect_capsem.containers.compose as c_compose_mod
import pytest
from inspect_capsem.containers.compose import ComposeInputs, ComposeLimits

LIMITS = ComposeLimits(65536, 4096, 64)


def test_compose_long_form_build_fields_and_refusals(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:

    def evaluator_inputs():
        dockerfile = tmp_path / "Dockerfile"
        files = {str(dockerfile): dockerfile.read_text()} if dockerfile.is_file() else {}
        return ComposeInputs(
            environment={"INHERITED_DICT_KEY": "inherited_val"},
            directories=frozenset({str(tmp_path / "src")}),
            files=files,
        )

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
        ext_long = c_compose_mod.extract_compose_fields(
            long_compose, base_dir=tmp_path, inputs=evaluator_inputs()
        )
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
    ext_build = c_compose_mod.extract_compose_fields(
        build_compose, base_dir=tmp_path, inputs=evaluator_inputs()
    )
    assert ext_build["build_args"] == {"FOO": "custom", "DEBUG": "true"}
    assert ext_build["build_target"] == "builder"
    with pytest.raises(ValueError, match="dockerfile_inline"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"build": {"dockerfile_inline": "FROM alpine\n"}}}},
            base_dir=tmp_path,
            inputs=evaluator_inputs(),
        )
    for bad_build_key in ("ssh", "secrets", "cache_from", "network", "platforms", "extra_hosts"):
        with pytest.raises(ValueError, match="Unsupported Compose build option"):
            c_compose_mod.extract_compose_fields(
                {"services": {"app": {"build": {"context": ".", bad_build_key: ["x"]}}}},
                base_dir=tmp_path,
                inputs=evaluator_inputs(),
            )
    with pytest.raises(ValueError, match="Multi-service Compose files are not supported"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"image": "app:1"}, "db": {"image": "postgres:16"}}},
            inputs=evaluator_inputs(),
        )
    with pytest.raises(ValueError, match="depends_on"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"image": "alpine", "depends_on": ["db"]}}},
            inputs=evaluator_inputs(),
        )
    with pytest.raises(ValueError, match="env_file"):
        c_compose_mod.extract_compose_fields(
            {"services": {"app": {"image": "alpine", "env_file": [".env"]}}},
            inputs=evaluator_inputs(),
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
            },
            inputs=evaluator_inputs(),
        )
    assert (
        "Ignoring unsupported compose keys in service 'app': cap_add, cpus, platform, privileged, shm_size, tmpfs"
        in caplog.text
    )
    assert "x-inspect" not in caplog.text
