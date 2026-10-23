"""Dispatch/adversarial unit proof with synthetic HTTP; never runtime evidence."""
from __future__ import annotations

import json
import tomllib
from types import SimpleNamespace

import pytest
import tomli_w
from capsem_builder import crypto
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_policy
from benchmarks.helpers.route_policy_contracts import corp_toml


class Backend:
    def __init__(self, home, cfg):
        self.home, self.cfg = home, cfg
        self.no_write = self.stale = self.refuse = self.vm = False
        self.calls = []
        condition = f'mcp.server.name == "{cfg["server_id"]}" && mcp.tool_call.name == "{cfg["tool_id"]}"'
        self.write({'settings': {cfg['setting_id']: {'value': cfg['setting_baseline'], 'modified': 'fixture'}},
            'plugins': {cfg['plugin_id']: {'mode': cfg['plugin_baseline'], 'detection_level': 'critical'}},
            'default': {'mcp': {'action': cfg['default_baseline']}},
            'profiles': {'rules': {'fixture': {'name': 'fixture', 'match': condition, 'action': cfg['tool_baseline'], 'enabled': True,
                'managed': {'category': 'mcp', 'target_kind': 'mcp_tool', 'target_key': 'local/fetch_http', 'operation': 'permission'}}}}})

    def write(self, document):
        (self.home / 'settings.toml').write_text(tomli_w.dumps(document))
        (self.home / 'settings.toml').chmod(0o600)

    def plugin(self, document):
        return {'id': self.cfg['plugin_id'], 'name': 'Synthetic fixture', 'description': 'Unit fixture only',
            'overridden': True, 'config': document['plugins'][self.cfg['plugin_id']],
            'runtime': {**dict.fromkeys(('event_count', 'execution_count', 'applied_count', 'skipped_count', 'total_duration_us', 'max_duration_us', 'detection_count', 'block_count', 'rewrite_count'), 0), 'enabled': True, 'brokered_credentials': []}}

    def event(self, before, after, kind, key, rule):
        return {'mutation_id': 'a' * 12, 'timestamp_unix_ms': 1, 'actor': 'service-api', 'category': 'mcp',
            'filename': 'settings.toml', 'affected_path': 'settings.toml', 'target_kind': kind, 'target_key': key,
            'operation': 'permission', 'rule_id': rule, 'old_hash': 'blake3:' + crypto.blake3(before).hexdigest(),
            'new_hash': 'blake3:' + crypto.blake3(after).hexdigest(), 'old_size': len(before), 'new_size': len(after), 'status': 'applied', 'error': None}

    def measure_request(self, method, path, *, body=None, **limits):
        self.calls.append((method, path))
        assert limits['timeout'] > 0 and limits['max_response_bytes'] > 0
        cfg = self.cfg
        before = (self.home / 'settings.toml').read_bytes()
        doc = tomllib.loads(before.decode())
        payload = None
        status = 200
        if method != 'GET':
            assert isinstance(body, bytes)
        if method == 'GET':
            if path == '/vms/list':
                payload = {'sandboxes': [{}] if self.vm else []}
            elif path == '/settings/info':
                value, source = doc['settings'][cfg['setting_id']]['value'], 'user'
                corp = self.home / 'corp.toml'
                if corp.exists():
                    value, source = tomllib.loads(corp.read_text())['settings'][cfg['setting_id']]['value'], 'corp'
                if self.stale:
                    value, source = cfg['setting_baseline'], 'user'
                payload = {'tree': [{'kind': 'leaf', 'id': cfg['setting_id'], 'effective_value': value, 'source': source, 'corp_locked': source == 'corp'}], 'issues': []}
            elif path == '/corp/info':
                corp = self.home / 'corp.toml'
                sidecar = self.home / 'corp-source.json'
                payload = {'installed': corp.exists(), 'paths': [{'path': str(corp), 'exists': corp.exists()}], 'source': json.loads(sidecar.read_text()) if sidecar.exists() else None}
            elif path.startswith('/plugins/'):
                payload = self.plugin(doc)
            elif path == '/mcp/default/info':
                payload = {'action': doc['default']['mcp']['action'], 'source': 'settings', 'rule_id': 'default.mcp'}
            elif path.endswith('/tools/list'):
                payload = [{'server_name': cfg['server_id'], 'original_name': cfg['tool_id'], 'permission_action': doc['profiles']['rules']['fixture']['action'], 'permission_source': 'settings'}]
        elif path == '/corp/reload':
            payload = {'success': True, 'reloaded': 0}
        elif path == '/corp/validate':
            assert body is not None
            assert json.loads(body)['toml'] == corp_toml(cfg)
            payload = {'success': True}
        elif path == '/corp/edit':
            assert body is not None
            content = json.loads(body)['toml']
            (self.home / 'corp.toml').write_text(content)
            (self.home / 'corp-source.json').write_text(json.dumps({'url': None, 'file_path': None, 'content_hash': crypto.blake3(content.encode()).hexdigest(), 'refresh_interval_hours': 24, 'fetched_at': 1}))
            payload = {'success': True}
        else:
            assert body is not None
            change = json.loads(body)
            if path == '/settings/edit':
                doc['settings'][cfg['setting_id']]['value'] = change[cfg['setting_id']]
                doc['settings'][cfg['setting_id']]['modified'] = '2026-10-23T22:00:00+00:00'
            elif path.startswith('/plugins/'):
                doc['plugins'][cfg['plugin_id']]['mode'] = change['mode']
            elif path == '/mcp/default/edit':
                doc['default']['mcp']['action'] = change['action']
            else:
                doc['profiles']['rules']['fixture']['action'] = change['action']
            if not self.no_write:
                self.write(doc)
            after = (self.home / 'settings.toml').read_bytes()
            if path == '/settings/edit':
                payload = {'tree': [{'kind': 'leaf', 'id': cfg['setting_id'], 'effective_value': cfg['setting_value'], 'source': 'user', 'corp_locked': False}], 'issues': []}
            elif path.startswith('/plugins/'):
                payload = self.plugin(doc)
            elif path == '/mcp/default/edit':
                payload = {'action': cfg['default_value'], 'mutation': self.event(before, after, 'mcp_default', 'default.mcp', 'default.mcp')}
            else:
                payload = {'server_id': cfg['server_id'], 'tool_id': cfg['tool_id'], 'action': cfg['tool_value'], 'mutation': self.event(before, after, 'mcp_tool', 'local/fetch_http', 'profiles.rules.fixture')}
            status = 403 if self.refuse else 200
        assert payload is not None, (method, path)
        return MeasuredResponse(status, {}, json.dumps(payload).encode(), 1, 2, 3)


@pytest.mark.parametrize('operation', list(route_policy.policy_config()['endpoints']))
def test_real_file_transition_and_public_readback_restore_exactly(tmp_path, operation):
    cfg = route_policy.policy_config()
    backend = Backend(tmp_path, cfg)
    original = route_policy.FileSnapshot(tmp_path, cfg['file_max_bytes'])
    target = SimpleNamespace(headers={})
    receipt, completion = route_policy.run_case(backend, target, tmp_path, operation, cfg)
    assert receipt.attempts == 1 and completion > 0
    original.assert_exact()
    assert ('POST', '/corp/reload') in backend.calls


@pytest.mark.parametrize('operation,failure', [(op, failure) for op in ('PATCH /settings/edit', 'PATCH /plugins/{plugin_id}/edit', 'PATCH /mcp/default/edit', 'PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit') for failure in ('no_write', 'refuse')] + [('PATCH /settings/edit', 'stale')])
def test_desired_echo_partial_failure_and_stale_readback_cannot_certify(tmp_path, operation, failure):
    cfg = route_policy.policy_config()
    backend = Backend(tmp_path, cfg)
    setattr(backend, failure, True)
    before = route_policy.FileSnapshot(tmp_path, cfg['file_max_bytes'])
    with pytest.raises((AssertionError, ValueError)):
        route_policy.run_case(backend, SimpleNamespace(headers={}), tmp_path, operation, cfg)
    before.assert_exact()


def test_host_only_case_rejects_running_owner_before_any_mutation(tmp_path):
    cfg = route_policy.policy_config()
    backend = Backend(tmp_path, cfg)
    backend.vm = True
    with pytest.raises(AssertionError, match='host-only'):
        route_policy.run_case(backend, SimpleNamespace(headers={}), tmp_path, 'PATCH /settings/edit', cfg)
    assert all(method == 'GET' for method, _ in backend.calls)


@pytest.mark.parametrize('operation', ['POST /corp/validate', 'PUT /corp/edit'])
def test_validation_write_or_corrupt_corp_provenance_fails_and_restores(monkeypatch, tmp_path, operation):
    cfg = route_policy.policy_config()
    backend = Backend(tmp_path, cfg)
    original = backend.measure_request
    def corrupt(method, path, **kwargs):
        result = original(method, path, **kwargs)
        if method == 'POST' and path == '/corp/validate':
            (tmp_path / 'corp.toml').write_text(corp_toml(cfg))
        elif method == 'PUT' and path == '/corp/edit':
            source = tmp_path / 'corp-source.json'
            payload = json.loads(source.read_text())
            payload['content_hash'] = 'corrupt'
            source.write_text(json.dumps(payload))
        return result
    monkeypatch.setattr(backend, 'measure_request', corrupt)
    before = route_policy.FileSnapshot(tmp_path, cfg['file_max_bytes'])
    with pytest.raises(AssertionError):
        route_policy.run_case(backend, SimpleNamespace(headers={}), tmp_path, operation, cfg)
    before.assert_exact()


def test_unrelated_policy_change_cannot_hide_behind_correct_setting_readback(monkeypatch, tmp_path):
    cfg = route_policy.policy_config()
    backend = Backend(tmp_path, cfg)
    original = backend.measure_request
    def broaden(method, path, **kwargs):
        result = original(method, path, **kwargs)
        if method == 'PATCH' and path == '/settings/edit':
            document = tomllib.loads((tmp_path / 'settings.toml').read_text())
            document['default']['mcp']['action'] = 'allow'
            backend.write(document)
        return result
    monkeypatch.setattr(backend, 'measure_request', broaden)
    before = route_policy.FileSnapshot(tmp_path, cfg['file_max_bytes'])
    with pytest.raises(AssertionError, match='unrelated'):
        route_policy.run_case(backend, SimpleNamespace(headers={}), tmp_path, 'PATCH /settings/edit', cfg)
    before.assert_exact()
