"""Independent file/public contracts for six finite host-only operations."""
from __future__ import annotations

import json
import tomllib
from datetime import datetime

from capsem_builder import crypto

from .route_policy import readback, tool_rule, validate_mcp_event, validate_setting
from .route_validation import plugin_info


def corp_toml(cfg):
    return f'refresh_policy = "24h"\n[settings."{cfg["setting_id"]}"]\nvalue = {cfg["corp_value"]}\nmodified = "route-fixture"\n'


def mutation_input(operation, cfg):
    method, path = operation.split(' ', 1)
    path = path.replace('{plugin_id}', cfg['plugin_id']).replace('{server_id}', cfg['server_id']).replace('{tool_id}', cfg['tool_id'])
    if path == '/settings/edit':
        body = {cfg['setting_id']: cfg['setting_value']}
    elif path in {'/corp/edit', '/corp/validate'}:
        body = {'toml': corp_toml(cfg)}
    elif operation == 'PATCH /plugins/{plugin_id}/edit':
        body = {'mode': cfg['plugin_value']}
    elif path == '/mcp/default/edit':
        body = {'action': cfg['default_value']}
    elif operation == 'PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit':
        body = {'action': cfg['tool_value']}
    else:
        raise ValueError('policy operation lacks a successful owning contract')
    assert method in {'PATCH', 'PUT', 'POST'}
    return method, path, body


def verify_mutation(operation, response, before, after, transport, target, cfg):
    before_settings = before.settings_bytes()
    after_settings = after.settings_bytes()
    document = tomllib.loads(after_settings.decode('utf-8'))
    if operation == 'POST /corp/validate':
        assert response == {'success': True}
        assert before.files == after.files, 'validation installed or modified policy'
        return
    if operation == 'PUT /corp/edit':
        assert response == {'success': True} and before_settings == after_settings
        saved = after.files['corp.toml']
        assert saved is not None and saved[0] == corp_toml(cfg).encode()
        provenance = after.files['corp-source.json']
        assert provenance is not None
        source = json.loads(provenance[0])
        # Existing corp provenance security format, through accelerated owner.
        assert source['content_hash'] == crypto.blake3(saved[0]).hexdigest()
        assert source.get('url') is None and source.get('file_path') is None
        assert source['refresh_interval_hours'] == 24 and source['fetched_at'] > 0
        info = readback(transport, target, '/corp/info', cfg)
        assert info['installed'] is True and info['source'] == source
        assert any(path['path'] == str(after.home / 'corp.toml') and path['exists'] is True for path in info['paths'])
        validate_setting(readback(transport, target, '/settings/info', cfg), cfg['setting_id'], cfg['corp_value'], 'corp')
        return
    assert before_settings != after_settings, 'mutation was a cached/no-op response'
    expected = tomllib.loads(before_settings.decode('utf-8'))
    if operation == 'PATCH /settings/edit':
        expected['settings'][cfg['setting_id']]['value'] = cfg['setting_value']
        modified = document['settings'][cfg['setting_id']]['modified']
        assert datetime.fromisoformat(modified).tzinfo is not None
        expected['settings'][cfg['setting_id']]['modified'] = modified
    elif operation == 'PATCH /plugins/{plugin_id}/edit':
        expected['plugins'][cfg['plugin_id']]['mode'] = cfg['plugin_value']
    elif operation == 'PATCH /mcp/default/edit':
        expected['default']['mcp']['action'] = cfg['default_value']
    else:
        tool_rule(expected, cfg)[1]['action'] = cfg['tool_value']
    assert document == expected, 'mutation changed unrelated policy'
    assert before.files['corp.toml'] == after.files['corp.toml'] is None
    assert before.files['corp-source.json'] == after.files['corp-source.json'] is None
    if operation == 'PATCH /settings/edit':
        assert document['settings'][cfg['setting_id']]['value'] == cfg['setting_value']
        validate_setting(response, cfg['setting_id'], cfg['setting_value'], 'user')
        validate_setting(readback(transport, target, '/settings/info', cfg), cfg['setting_id'], cfg['setting_value'], 'user')
    elif operation == 'PATCH /plugins/{plugin_id}/edit':
        assert document['plugins'][cfg['plugin_id']]['mode'] == cfg['plugin_value']
        plugin_info(response, cfg['plugin_id'])
        info = readback(transport, target, f'/plugins/{cfg["plugin_id"]}/info', cfg)
        plugin_info(info, cfg['plugin_id'])
        assert response['config'] == info['config'] and info['config']['mode'] == cfg['plugin_value']
        assert info['overridden'] is True
    elif operation == 'PATCH /mcp/default/edit':
        assert response['action'] == document['default']['mcp']['action'] == cfg['default_value']
        assert readback(transport, target, '/mcp/default/info', cfg) == {'action': cfg['default_value'], 'source': 'settings', 'rule_id': 'default.mcp'}
        validate_mcp_event(response['mutation'], before_settings, after_settings, 'mcp_default', 'default.mcp', 'default.mcp')
    else:
        key, rule = tool_rule(document, cfg)
        assert response['server_id'] == cfg['server_id'] and response['tool_id'] == cfg['tool_id']
        assert response['action'] == rule['action'] == cfg['tool_value']
        (tool,) = readback(transport, target, f'/mcp/servers/{cfg["server_id"]}/tools/list', cfg)
        assert tool['original_name'] == cfg['tool_id'] and tool['server_name'] == cfg['server_id']
        assert tool['permission_action'] == cfg['tool_value'] and tool['permission_source'] == 'settings'
        validate_mcp_event(response['mutation'], before_settings, after_settings, 'mcp_tool', f'{cfg["server_id"]}/{cfg["tool_id"]}', f'profiles.rules.{key}')
