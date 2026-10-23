"""Finite policy cases must prove persisted host state and exact restore."""
from __future__ import annotations

from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_policy


def test_policy_cases_bind_exact_six_mutation_methods():
    cfg = route_policy.policy_config()
    assert set(cfg['endpoints']) == {'PATCH /settings/edit', 'PUT /corp/edit', 'POST /corp/validate', 'PATCH /plugins/{plugin_id}/edit', 'PATCH /mcp/default/edit', 'PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit'}
    assert 0 < cfg['quick_cycles'] < cfg['cycles'] <= 10
    assert cfg['setting_id'] == 'appearance.font_size'


def test_snapshot_restore_is_exact_and_bounded_including_absence(monkeypatch, tmp_path):
    settings = tmp_path / 'settings.toml'
    settings.write_bytes(b'original\n')
    settings.chmod(0o600)
    saved = route_policy.FileSnapshot(tmp_path, 100)
    settings.write_bytes(b'changed\n')
    (tmp_path / 'corp.toml').write_bytes(b'new')
    saved.restore()
    assert settings.read_bytes() == b'original\n' and settings.stat().st_mode & 0o777 == 0o600
    assert not (tmp_path / 'corp.toml').exists()
    settings.write_bytes(b'x' * 101)
    with pytest.raises(ValueError, match='budget'):
        route_policy.FileSnapshot(tmp_path, 100)
    settings.unlink()
    settings.symlink_to(tmp_path / 'outside')
    with pytest.raises(ValueError, match='symlink'):
        route_policy.FileSnapshot(tmp_path, 100)


def test_settings_readback_rejects_desired_echo_wrong_source_or_empty_tree():
    cfg = route_policy.policy_config()
    leaf = {'kind': 'leaf', 'id': cfg['setting_id'], 'effective_value': cfg['setting_value'], 'source': 'user', 'corp_locked': False}
    payload = {'tree': [leaf], 'issues': []}
    route_policy.validate_setting(payload, cfg['setting_id'], cfg['setting_value'], 'user')
    for changed in ({'effective_value': cfg['setting_baseline']}, {'source': 'default'}, {'corp_locked': True}):
        with pytest.raises(AssertionError):
            route_policy.validate_setting({'tree': [{**leaf, **changed}], 'issues': []}, cfg['setting_id'], cfg['setting_value'], 'user')
    with pytest.raises(AssertionError):
        route_policy.validate_setting({'tree': [], 'issues': []}, cfg['setting_id'], cfg['setting_value'], 'user')


def test_mcp_mutation_receipt_matches_independent_before_after_file_bytes():
    from capsem_builder import crypto
    before, after = b'old', b'changed'
    event = {'mutation_id': 'a' * 12, 'timestamp_unix_ms': 1, 'actor': 'service-api', 'category': 'mcp', 'filename': 'settings.toml', 'affected_path': 'settings.toml', 'target_kind': 'mcp_default', 'target_key': 'default.mcp', 'operation': 'permission', 'rule_id': 'default.mcp', 'old_hash': 'blake3:' + crypto.blake3(before).hexdigest(), 'new_hash': 'blake3:' + crypto.blake3(after).hexdigest(), 'old_size': len(before), 'new_size': len(after), 'status': 'applied', 'error': None, 'trace_id': None}
    route_policy.validate_mcp_event(event, before, after, 'mcp_default', 'default.mcp', 'default.mcp')
    for changed in ({'new_hash': event['old_hash']}, {'status': 'failed'}, {'target_key': 'other'}, {'new_size': 0}):
        with pytest.raises(AssertionError):
            route_policy.validate_mcp_event({**event, **changed}, before, after, 'mcp_default', 'default.mcp', 'default.mcp')


def test_mutations_do_not_use_functional_retries_or_accept_http_refusal():
    from helpers.http_transport import Transport

    from benchmarks.helpers.route_requests import Target, json_request
    target = Target('service', lambda: Transport(host='127.0.0.1', port=1), {}, {})
    receipt = MeasuredResponse(200, {}, b'{"success":true}', 1, 2, 3)
    strict = SimpleNamespace(measure_request=lambda *a, **kw: receipt)
    measured, body = json_request(strict, target, 'PUT', '/corp/edit', {'toml': ''}, timeout=1, maximum=100)
    assert measured is receipt and body == {'success': True}
    for bad in (MeasuredResponse(403, {}, b'{}', 1, 2, 3), MeasuredResponse(200, {}, b'{}', 1, 2, 3, attempts=2)):
        with pytest.raises((AssertionError, ValueError)):
            json_request(SimpleNamespace(measure_request=lambda *a, value=bad, **kw: value), target, 'PATCH', '/settings/edit', {}, timeout=1, maximum=100)


def test_policy_counts_reject_partial_receipts_and_noop_config():
    from capsem_builder.gate.benchmarkpolicyschema import PolicyRouteConfig
    cfg = route_policy.policy_config()
    PolicyRouteConfig.model_validate(cfg)
    with pytest.raises(ValueError):
        PolicyRouteConfig.model_validate({**cfg, 'setting_value': cfg['setting_baseline']})
    with pytest.raises(ValueError):
        PolicyRouteConfig.model_validate({**cfg, 'cycles': 100})
    metrics = {}
    for label in ('service', 'gateway'):
        for operation, entry in cfg['endpoints'].items():
            (case,) = entry['cases']
            prefix = f'{label}.{operation} [{case}]'
            for suffix in ('latency_ms', 'headers_ms', 'first_body_byte_ms', 'verified_host_completion_ms'):
                metrics[prefix + '.' + suffix] = {'samples': [1.0] * cfg['quick_cycles']}
            for cycle in range(cfg['quick_cycles']):
                metrics[f'{prefix}.resources.{cycle}.wall_s'] = {'samples': [1.0]}
    route_policy.require_policy_metrics(metrics, cfg, True)
    del metrics['gateway.PATCH /settings/edit [host-setting-changed].first_body_byte_ms']
    with pytest.raises(ValueError, match='partial'):
        route_policy.require_policy_metrics(metrics, cfg, True)
