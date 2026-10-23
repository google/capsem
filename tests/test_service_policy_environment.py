"""The existing service owner can isolate corp inputs without global env edits."""
from __future__ import annotations

from types import SimpleNamespace

import pytest

from tests.helpers import service as helper


@pytest.mark.parametrize('isolated', [False, True])
def test_isolated_corp_input_only_changes_child_environment(monkeypatch, tmp_path, isolated):
    home = tmp_path / 'home'
    home.mkdir()
    monkeypatch.setattr(helper, 'make_capsem_tmp_dir', lambda _: home)
    monkeypatch.setenv('CAPSEM_CORP_CONFIG', '/outside/never-open.toml')
    captured = []
    proc = SimpleNamespace(poll=lambda: None, terminate=lambda: None, wait=lambda **_: 0)
    monkeypatch.setattr(helper.subprocess, 'Popen', lambda *_, **kwargs: (captured.append(kwargs['env']) or proc))
    monkeypatch.setattr(helper, 'Transport', lambda **_: SimpleNamespace(request=lambda *a, **kw: (200, b'{}'), close=lambda: None))
    service = helper.ServiceInstance(sign_binaries=False, isolated_corp_config=isolated)
    service.uds_path.touch()
    try:
        service.start()
        expected = str(home / 'corp.toml') if isolated else '/outside/never-open.toml'
        assert captured[0]['CAPSEM_CORP_CONFIG'] == expected
        assert helper.os.environ['CAPSEM_CORP_CONFIG'] == '/outside/never-open.toml'
        assert captured[0]['CAPSEM_HOME'] == str(home)
        assert captured[0]['CAPSEM_RUN_DIR'] == str(home / 'run')
    finally:
        service.stop()
