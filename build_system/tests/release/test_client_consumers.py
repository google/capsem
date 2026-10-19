"""Public consumer configuration cannot borrow publisher or workspace settings."""

import ast

from capsem_builder.release.tools.client_consumer_probes import PYTHON
from capsem_builder.release.tools.client_consumers import clean_environment


def test_consumers_discard_authentication_and_local_dependency_overrides() -> None:
    denied = {
        "PYPI_TOKEN", "NPM_TOKEN", "NODE_AUTH_TOKEN", "CARGO_REGISTRY_TOKEN", "UV_PUBLISH_TOKEN",
        "UV_INDEX", "UV_INDEX_URL", "UV_OFFLINE", "PIP_INDEX_URL", "GITHUB_TOKEN", "GH_TOKEN",
        "NPM_CONFIG_REGISTRY", "npm_config_userconfig", "CARGO_REGISTRIES_CRATES_IO_INDEX",
        "PYTHONPATH", "PYTHONHOME", "NODE_PATH", "NODE_OPTIONS", "VIRTUAL_ENV",
        "CAPSEM_GATEWAY_URL", "CAPSEM_GATEWAY_TOKEN",
    }
    source = dict.fromkeys(denied, "must-not-reach-consumer")
    source.update(PATH="/bin", CAPSEM_RUN_DIR="/private/fixture/run")
    assert clean_environment(source) == {"PATH": "/bin", "CAPSEM_RUN_DIR": "/private/fixture/run"}
    assert denied <= source.keys(), "cleaning must not mutate publisher environment"


def test_isolated_python_probe_is_valid_python() -> None:
    ast.parse(PYTHON)
