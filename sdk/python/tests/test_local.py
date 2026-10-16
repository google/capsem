"""Local credentials stay local; remote clients never discover them."""

import asyncio
from pathlib import Path

import pytest
from capsem import Hypervisor

from .facade_gateway import gateway


def metadata(root: Path, port: str = "19222", token: str = "token") -> None:
    root.mkdir(parents=True, exist_ok=True)
    (root / "gateway.port").write_text(port)
    (root / "gateway.token").write_text(token)
    (root / "gateway.token").chmod(0o600)


def test_no_arguments_use_the_local_gateway(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    async def run() -> None:
        async with gateway() as (url, state):
            metadata(tmp_path, url.rsplit(":", 1)[1])
            monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
            async with Hypervisor() as hv:
                await hv.list()
            assert [path for _, path, _ in state.requests] == ["/vms/list"]
    asyncio.run(run())


@pytest.mark.parametrize("override", ["HOME", "CAPSEM_HOME", "CAPSEM_RUN_DIR"])
def test_path_precedence(override: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("CAPSEM_RUN_DIR", raising=False)
    monkeypatch.delenv("CAPSEM_HOME", raising=False)
    monkeypatch.setenv("HOME", str(tmp_path / "missing"))
    monkeypatch.setenv(override, str(tmp_path))
    root = tmp_path / ".capsem/run" if override == "HOME" else tmp_path / "run" if override == "CAPSEM_HOME" else tmp_path
    metadata(root)
    hv = Hypervisor()
    asyncio.run(hv.close())


@pytest.mark.parametrize("port", ["0", "65536", "-1", "1.5", "19222/evil", "evil.example:80", "1\n2", "", "1" * 5000])
def test_bad_port_is_rejected(port: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
    metadata(tmp_path, port)
    with pytest.raises(ConnectionError, match="local Capsem"):
        Hypervisor()


@pytest.mark.parametrize("token", ["", "a\nb", "a\rb", "x" * 5000])
def test_bad_token_is_not_in_error(token: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
    metadata(tmp_path, token=token)
    with pytest.raises(ConnectionError, match="local Capsem") as caught:
        Hypervisor()
    if token:
        assert token not in str(caught.value)


def test_missing_private_or_symlinked_token_is_refused(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
    with pytest.raises(ConnectionError, match="Start Capsem"):
        Hypervisor()
    metadata(tmp_path)
    token = tmp_path / "gateway.token"
    token.chmod(0o644)
    with pytest.raises(ConnectionError):
        Hypervisor()
    token.rename(tmp_path / "secret")
    token.symlink_to(tmp_path / "secret")
    with pytest.raises(ConnectionError):
        Hypervisor()


def test_explicit_clients_do_not_read_local_credentials(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path / "missing"))
    hv = Hypervisor("https://remote.example", "remote-secret")
    asyncio.run(hv.close())
    with pytest.raises(ValueError, match="both"):
        Hypervisor(url="https://remote.example")
    with pytest.raises(ValueError, match="both"):
        Hypervisor(token="remote-secret")


def test_new_clients_read_rotated_metadata(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path))
    metadata(tmp_path)
    first = Hypervisor()
    metadata(tmp_path, token="rotated")
    second = Hypervisor()
    assert first._transport._token == "token"
    assert second._transport._token == "rotated"
    asyncio.run(first.close())
    asyncio.run(second.close())
