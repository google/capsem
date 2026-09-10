"""Guest launcher policy, exercised without executing an image on the host."""

import importlib.util
from pathlib import Path

import pytest

SOURCE = Path(__file__).resolve().parents[3] / "guest/artifacts/container/launch.py"


@pytest.fixture
def launcher():
    spec = importlib.util.spec_from_file_location("container_launch", SOURCE)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def image():
    return {
        "config": {
            "Entrypoint": ["docker-entrypoint.sh"],
            "Cmd": ["redis-server"],
            "Volumes": {"/data": {}},
        }
    }


def unpacked():
    return {
        "process": {
            "args": ["docker-entrypoint.sh", "redis-server"],
            "env": ["PATH=/usr/local/bin:/usr/bin:/bin", "A=old"],
            "cwd": "/data",
            "user": {"uid": 0, "gid": 0},
        },
        "hooks": {"prestart": [{"path": "/evil"}]},
    }


def test_default_command_user_and_workdir_survive_hardening(launcher):
    config = launcher.configure(unpacked(), image(), {"args": [], "env": {}})
    process = config["process"]
    assert process["args"] == ["docker-entrypoint.sh", "redis-server"]
    assert process["user"] == {"uid": 0, "gid": 0}
    assert process["cwd"] == "/data"
    assert process["noNewPrivileges"] is True
    assert config["root"] == {"path": "rootfs", "readonly": True}
    hooks = config["hooks"]["prestart"]
    assert len(hooks) == 1 and hooks[0]["path"] == "/usr/bin/python3"
    assert hooks[0]["args"] == ["/usr/bin/python3", str(SOURCE), "--network-ready"]
    assert hooks[0]["timeout"] == 5
    assert {ns["type"] for ns in config["linux"]["namespaces"]} == {
        "pid",
        "mount",
        "ipc",
        "uts",
        "network",
    }
    assert "CAP_SYS_ADMIN" not in process["capabilities"]["bounding"]
    assert "CAP_NET_RAW" not in process["capabilities"]["bounding"]
    assert config["linux"]["resources"]["memory"]["limit"] == 256 * 1024**2
    assert config["linux"]["resources"]["pids"]["limit"] == 256
    assert all(mount["type"] in {"proc", "tmpfs"} for mount in config["mounts"])
    assert "/data" in {mount["destination"] for mount in config["mounts"]}


def test_command_override_replaces_cmd_but_preserves_entrypoint(launcher):
    options = {"args": ["redis-server", "--save", ""], "env": {"A": "new", "B": "a=b"}}
    config = launcher.configure(unpacked(), image(), options)
    assert config["process"]["args"] == [
        "docker-entrypoint.sh",
        "redis-server",
        "--save",
        "",
    ]
    assert config["process"]["env"] == [
        "PATH=/usr/local/bin:/usr/bin:/bin",
        "A=new",
        "B=a=b",
    ]


@pytest.mark.parametrize(
    "volume", ["/", "../escape", "/data/../etc", "/proc", "/dev/x", "/sys", "/usr/bin"]
)
def test_image_cannot_replace_security_mounts_or_root(launcher, volume):
    source = image()
    source["config"]["Volumes"] = {volume: {}}
    with pytest.raises(ValueError):
        launcher.configure(unpacked(), source, {"args": [], "env": {}})


def test_missing_command_fails_before_runtime_launch(launcher):
    config = unpacked()
    config["process"]["args"] = []
    with pytest.raises(ValueError, match="command"):
        launcher.configure(config, {"config": {}}, {"args": [], "env": {}})
