"""Public consumer configuration cannot borrow publisher or workspace settings."""

import ast
import json
from pathlib import Path

import pytest
from capsem_builder.gate.config import load
from capsem_builder.gate.versions import workspace_version
from capsem_builder.release.tools import client_consumers
from capsem_builder.release.tools.client_consumer_probes import PYTHON
from capsem_builder.release.tools.client_consumers import clean_environment


def test_consumers_discard_authentication_and_local_dependency_overrides() -> None:
    denied = {
        "PYPI_TOKEN", "NPM_TOKEN", "NODE_AUTH_TOKEN", "CARGO_REGISTRY_TOKEN", "UV_PUBLISH_TOKEN",
        "UV_INDEX", "UV_INDEX_URL", "UV_OFFLINE", "PIP_INDEX_URL", "GITHUB_TOKEN", "GH_TOKEN",
        "NPM_CONFIG_REGISTRY", "npm_config_userconfig", "CARGO_REGISTRIES_CRATES_IO_INDEX",
        "PYTHONPATH", "PYTHONHOME", "NODE_PATH", "NODE_OPTIONS", "VIRTUAL_ENV",
        "CAPSEM_GATEWAY_URL", "CAPSEM_GATEWAY_TOKEN",
        "ACTIONS_ID_TOKEN_REQUEST_URL", "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    }
    source = dict.fromkeys(denied, "must-not-reach-consumer")
    source.update(PATH="/bin", CAPSEM_RUN_DIR="/private/fixture/run")
    assert clean_environment(source) == {"PATH": "/bin", "CAPSEM_RUN_DIR": "/private/fixture/run"}
    assert denied <= source.keys(), "cleaning must not mutate publisher environment"


def test_isolated_python_probe_is_valid_python() -> None:
    ast.parse(PYTHON)


def test_public_consumers_install_only_the_enabled_registries(monkeypatch: pytest.MonkeyPatch) -> None:
    config = load(Path(__file__).resolve().parents[3])
    version = workspace_version(config.root)
    commands = []

    def run(argv, *, cwd, env, **_kwargs):
        commands.append(argv)
        assert argv[0] != "cargo", "Cargo registry publication and consumption are deferred"
        assert "ACTIONS_ID_TOKEN_REQUEST_TOKEN" not in env
        if argv[:2] == ["npm", "install"]:
            packages = {f"node_modules/@capsem/{name}": {
                "version": version, "resolved": f"https://registry.npmjs.org/@capsem/{name}/fixture.tgz",
            } for name in ("sdk", "mcp")}
            (cwd / "package-lock.json").write_text(json.dumps({"packages": packages}))

    monkeypatch.setattr(client_consumers.subprocess, "run", run)
    client_consumers.verify(config, {"PATH": "/bin", "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "publisher-only"})
    assert any(argv[:3] == ["uv", "pip", "install"] for argv in commands)
    assert any(argv[:2] == ["npm", "install"] for argv in commands)
    assert ["node", "probe.mjs"] in commands
