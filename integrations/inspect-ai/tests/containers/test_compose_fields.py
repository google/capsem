"""Pure cases carried from Pierre Tholoniat's original bb61fc82d tests."""

from pathlib import Path

import inspect_capsem.containers.compose as c_compose_mod
import pytest
from inspect_capsem.containers.compose import ComposeInputs, ComposeLimits

LIMITS = ComposeLimits(65536, 4096, 64)


def test_compose_field_extraction(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    inputs = ComposeInputs(
        environment={"INHERITED_VAR": "from_os"}, directories=frozenset({str(tmp_path / "ctx")})
    )
    extract = c_compose_mod.extract_compose_fields
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        extract({"services": {}}, inputs=inputs)
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        extract({"execution_mode": "vm"}, inputs=inputs)
    ctx = tmp_path / "ctx"
    ctx.mkdir()
    with pytest.raises(ValueError, match="Multi-service Compose files are not supported"):
        extract(
            {"services": {"a": {"image": "img:a"}, "b": {"image": "img:b"}}},
            base_dir=tmp_path,
            inputs=inputs,
        )
    services = {"b": {"image": None, "build": "ctx", "working_dir": "/w"}}
    fields = extract({"services": services}, base_dir=tmp_path, inputs=inputs)
    assert fields == {
        "execution_mode": "container",
        "dockerfile": str(ctx / "Dockerfile"),
        "working_dir": "/w",
    }
    services = {"default": {"build": "Dockerfile.x"}}
    assert extract({"services": services}, inputs=inputs)["dockerfile"] == "Dockerfile.x"
    build = {"context": "sub", "dockerfile": "nested/Custom"}
    services = {"svc": {"build": build}}
    sub_fields = extract({"services": services}, base_dir=tmp_path, inputs=inputs)
    assert sub_fields["dockerfile"] == str(tmp_path / "sub" / "nested" / "Custom")
    assert sub_fields["build_context"] == str(tmp_path / "sub")
    services = {"svc": {"build": {"context": "."}}}
    assert extract({"services": services}, inputs=inputs)["dockerfile"] == "Dockerfile"
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
    rich = extract(rich_compose, base_dir=tmp_path, inputs=inputs)
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
