"""Fixture ownership/dispatch tests, with service and HTTP launches replaced."""
from __future__ import annotations

import json
import tomllib
from types import SimpleNamespace

from benchmarks.helpers import route_policy_fixture as fixture
from benchmarks.helpers.route_policy import policy_config


def test_policy_fixture_isolates_corp_and_never_approves_or_invokes_tools(monkeypatch, tmp_path):
    cfg = policy_config()
    home = tmp_path / 'owned'
    home.mkdir()
    source = tmp_path / 'assets'
    source.mkdir()
    (source / 'manifest.json').write_text('{}')
    calls = []
    def start():
        document = tomllib.loads((home / 'settings.toml').read_text())
        assert 'default' not in document, 'MCP defaults must be initialized by their canonical route'
        calls.append('start')
    service = SimpleNamespace(home_dir=home, start=start, stop=lambda: calls.append('stop'))
    def create(**kwargs):
        assert kwargs == {'isolated_corp_config': True}
        return service
    monkeypatch.setattr(fixture, 'ASSETS_DIR', source)
    monkeypatch.setattr(fixture, 'ServiceInstance', create)
    transport = SimpleNamespace(close=lambda: calls.append('close'))
    monkeypatch.setattr(fixture, 'service_target', lambda owner, label: SimpleNamespace(transport_factory=lambda: transport))
    expected = [('/mcp/default/edit', {'action': cfg['default_baseline']}),
                ('/mcp/servers/local/tools/fetch_http/edit', {'action': cfg['tool_baseline']})]
    initialized = []
    def initialize(_, __, method, path, config, body):
        assert method == 'PATCH' and (path, body) == expected[len(initialized)]
        initialized.append((path, body))
        calls.append('initial-permission')
        return None, body
    monkeypatch.setattr(fixture, 'call', initialize)
    monkeypatch.setattr(fixture, 'validate_baseline', lambda *args: calls.append('baseline'))
    with fixture.policy_target('gateway', cfg) as (_, owner):
        assert owner is service
        document = tomllib.loads((home / 'settings.toml').read_text())
        assert document['settings'][cfg['setting_id']]['value'] == cfg['setting_baseline']
        assert document['settings']['app.auto_update']['value'] is False
        assert document['images']['catalog'] is False
        assert not (home / 'corp.toml').exists()
        cache = json.loads((home / 'mcp_tool_cache.json').read_text())
        assert all(tool['approved'] is False and tool['server_name'] == 'local' for tool in cache)
    assert initialized == expected
    assert calls == ['start', 'initial-permission', 'initial-permission', 'baseline', 'close', 'stop']
