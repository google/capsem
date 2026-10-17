"""Connection discovery never combines a remote destination with local secrets."""

from collections.abc import Callable
from pathlib import Path

import pytest
from capsem import VM, Hypervisor, discover_gateway

from .test_local import metadata


@pytest.mark.parametrize("client", [discover_gateway, Hypervisor, Hypervisor.connect, VM])
@pytest.mark.parametrize("explicit", [{"url": "https://remote.example"}, {"token": "remote-secret"}])
def test_partial_explicit_connections_cannot_borrow_local_or_environment_credentials(
    client: Callable[..., object], explicit: dict[str, str], tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    metadata(tmp_path)
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
    monkeypatch.setenv("CAPSEM_GATEWAY_URL", "https://another.example")
    monkeypatch.setenv("CAPSEM_GATEWAY_TOKEN", "environment-secret")
    with pytest.raises(ValueError, match="both"):
        client(**explicit)


@pytest.mark.parametrize("variable", ["CAPSEM_GATEWAY_URL", "CAPSEM_GATEWAY_TOKEN"])
def test_partial_environment_pair_cannot_borrow_local_metadata(
    variable: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    metadata(tmp_path)
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
    monkeypatch.delenv("CAPSEM_GATEWAY_URL", raising=False)
    monkeypatch.delenv("CAPSEM_GATEWAY_TOKEN", raising=False)
    monkeypatch.setenv(variable, "https://remote.example" if variable.endswith("URL") else "environment-secret")
    with pytest.raises(ValueError, match="both"):
        discover_gateway()


def test_discovered_endpoint_repr_never_contains_the_token() -> None:
    endpoint = discover_gateway(url="https://remote.example", token="private-token-for-repr")
    assert endpoint.token == "private-token-for-repr"
    assert endpoint.token not in repr(endpoint)


def test_missing_port_cannot_fall_back_to_another_gateway(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    metadata(tmp_path)
    (tmp_path / "gateway.port").unlink()
    monkeypatch.delenv("CAPSEM_GATEWAY_URL", raising=False)
    monkeypatch.delenv("CAPSEM_GATEWAY_TOKEN", raising=False)
    with pytest.raises(ConnectionError, match="local Capsem"):
        discover_gateway(run_dir=tmp_path)
