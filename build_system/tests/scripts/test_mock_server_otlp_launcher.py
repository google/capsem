"""The synthetic collector is enabled only through an explicit fixture option."""
from types import SimpleNamespace

import pytest

from build_system.tests.helpers import mock_server


@pytest.mark.parametrize("enabled", [False, True])
def test_otlp_fixture_flag_is_explicit(monkeypatch, enabled):
    commands = []
    monkeypatch.setattr(mock_server, "_ensure_mock_server_binary", lambda: None)
    monkeypatch.setattr(mock_server, "read_ready_json", lambda _: {"service": "capsem-mock-server"})
    def launch(command, **_):
        commands.append(command)
        return SimpleNamespace()
    monkeypatch.setattr(mock_server, "_MockServerProcess", launch)
    mock_server.start_mock_server(addr="127.0.0.1:0", capture_requests=False, otlp_http_fixture=enabled)
    assert ("--otlp-http-fixture" in commands[0]) is enabled


@pytest.mark.parametrize("case", ["valid", "empty", "canary"])
def test_native_privacy_reads_the_real_gateway_log_owner(tmp_path, case):
    from tests.ironbank.kingslanding.test_gateway_telemetry import _private_logs

    stopped = []
    service = SimpleNamespace(tmp_dir=tmp_path, stop=lambda **kwargs: stopped.append(kwargs))
    (tmp_path / "service.log").write_text("service lifecycle\n")
    gateway = "gateway lifecycle\n" if case == "valid" else ""
    if case == "canary":
        gateway = "private-path-canary\n"
    (tmp_path / "gateway.log").write_text(gateway)
    if case == "valid":
        assert _private_logs(service) == "service lifecycle\ngateway lifecycle\n"
    else:
        with pytest.raises(AssertionError):
            _private_logs(service)
    assert stopped == [{"cleanup": False}]
