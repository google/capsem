"""Finite host-persisted policy cases; no VM-publication or physical-ledger claim."""
from __future__ import annotations

import stat
import time
import tomllib
from functools import partial
from pathlib import Path

from capsem_builder import crypto

from .route_measurements import Observations
from .route_requests import json_request
from .stream_measurements import measure, sample_count

ROOT = Path(__file__).resolve().parents[2]
FILES = ('settings.toml', 'corp.toml', 'corp-source.json')


def policy_config() -> dict:
    from capsem_builder.gate.benchmarkpolicyschema import PolicyRouteConfig
    raw = tomllib.loads((ROOT / 'config/gate.toml').read_text())['benchmark']['routes']['policy']
    return PolicyRouteConfig.model_validate(raw).model_dump()


class FileSnapshot:
    """Bounded exact fixture state, including file absence and permissions."""
    def __init__(self, home: Path, maximum: int):
        self.home, self.maximum = home, maximum
        self.files: dict[str, tuple[bytes, int] | None] = {}
        remaining = maximum
        for name in FILES:
            path = home / name
            if path.is_symlink():
                raise ValueError('policy fixture refuses symlink input')
            if not path.exists():
                self.files[name] = None
                continue
            with path.open('rb') as source:
                body = source.read(remaining + 1)
            if len(body) > remaining:
                raise ValueError('policy fixture file budget exceeded')
            remaining -= len(body)
            self.files[name] = (body, stat.S_IMODE(path.stat().st_mode))

    def restore(self) -> None:
        for name, saved in self.files.items():
            path = self.home / name
            if path.is_symlink():
                raise ValueError('policy fixture refuses symlink restore')
            if saved is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(saved[0])
                path.chmod(saved[1])
        self.assert_exact()

    def assert_exact(self) -> None:
        assert FileSnapshot(self.home, self.maximum).files == self.files, 'policy fixture state was not restored exactly'

    def settings_bytes(self) -> bytes:
        saved = self.files['settings.toml']
        assert saved is not None
        return saved[0]


def validate_setting(payload, setting_id, value, source):
    def leaves(nodes):
        for node in nodes:
            if node.get('kind') == 'leaf':
                yield node
            yield from leaves(node.get('children', []))
    assert isinstance(payload['issues'], list)
    matches = [node for node in leaves(payload['tree']) if node.get('id') == setting_id]
    assert len(matches) == 1, 'missing or ambiguous effective setting'
    leaf = matches[0]
    assert leaf['effective_value'] == value and type(leaf['effective_value']) is type(value)
    assert leaf['source'] == source and leaf['corp_locked'] is (source == 'corp')


def validate_mcp_event(event, before, after, kind, key, rule_id):
    # These hashes are the existing security-audit wire identity, not a new
    # deduplication step. Verify outside request clocks through the crypto owner.
    assert before != after
    expected = {'actor': 'service-api', 'category': 'mcp', 'filename': 'settings.toml', 'affected_path': 'settings.toml',
        'target_kind': kind, 'target_key': key, 'operation': 'permission', 'rule_id': rule_id,
        'old_hash': 'blake3:' + crypto.blake3(before).hexdigest(), 'new_hash': 'blake3:' + crypto.blake3(after).hexdigest(),
        'old_size': len(before), 'new_size': len(after), 'status': 'applied', 'error': None}
    assert all(event[name] == value for name, value in expected.items())
    assert len(event['mutation_id']) == 12 and event['timestamp_unix_ms'] > 0


def call(transport, target, method, path, cfg, body=None):
    return json_request(transport, target, method, path, body,
        timeout=cfg['timeout_seconds'], maximum=cfg['max_response_bytes'])


def readback(transport, target, path, cfg):
    return call(transport, target, 'GET', path, cfg)[1]


def tool_rule(document, cfg):
    condition = f'mcp.server.name == "{cfg["server_id"]}" && mcp.tool_call.name == "{cfg["tool_id"]}"'
    (entry,) = [(key, rule) for key, rule in document['profiles']['rules'].items() if rule['match'] == condition]
    key, rule = entry
    assert rule['name'] == key and rule.get('enabled', True) is True and rule.get('managed')
    return key, rule


def validate_baseline(transport, target, home, cfg):
    assert readback(transport, target, '/vms/list', cfg)['sandboxes'] == [], 'host-only case acquired a VM'
    validate_setting(readback(transport, target, '/settings/info', cfg), cfg['setting_id'], cfg['setting_baseline'], 'user')
    assert readback(transport, target, '/corp/info', cfg)['installed'] is False
    plugin = readback(transport, target, f'/plugins/{cfg["plugin_id"]}/info', cfg)
    assert plugin['id'] == cfg['plugin_id'] and plugin['config']['mode'] == cfg['plugin_baseline']
    default = readback(transport, target, '/mcp/default/info', cfg)
    assert default == {'action': cfg['default_baseline'], 'source': 'settings', 'rule_id': 'default.mcp'}
    (tool,) = readback(transport, target, f'/mcp/servers/{cfg["server_id"]}/tools/list', cfg)
    assert tool['original_name'] == cfg['tool_id'] and tool['permission_action'] == cfg['tool_baseline'] and tool['permission_source'] == 'settings'
    before = FileSnapshot(home, cfg['file_max_bytes'])
    document = tomllib.loads(before.settings_bytes().decode('utf-8'))
    assert document['settings'][cfg['setting_id']]['value'] == cfg['setting_baseline']
    assert document['plugins'][cfg['plugin_id']]['mode'] == cfg['plugin_baseline']
    assert document['default']['mcp']['action'] == cfg['default_baseline']
    assert tool_rule(document, cfg)[1]['action'] == cfg['tool_baseline']
    assert before.files['corp.toml'] is None and before.files['corp-source.json'] is None
    return before


def run_case(transport, target, home, operation, cfg):
    """Return one strict receipt and independently verified host completion time."""
    from .route_policy_contracts import mutation_input, verify_mutation
    before = validate_baseline(transport, target, home, cfg)
    method, path, body = mutation_input(operation, cfg)
    try:
        started = time.monotonic_ns()
        receipt, response = call(transport, target, method, path, cfg, body)
        after = FileSnapshot(home, cfg['file_max_bytes'])
        verify_mutation(operation, response, before, after, transport, target, cfg)
        assert readback(transport, target, '/vms/list', cfg)['sandboxes'] == []
        completed = (time.monotonic_ns() - started) / 1_000_000
        return receipt, completed
    finally:
        before.restore()
        _, reloaded = call(transport, target, 'POST', '/corp/reload', cfg, {})
        assert reloaded == {'success': True, 'reloaded': 0}, 'restore unexpectedly published to a VM'
        before.assert_exact()
        validate_baseline(transport, target, home, cfg)


def collect_policy(quick: bool) -> dict:
    from psutil import Process

    from .route_policy_fixture import policy_target
    cfg = policy_config()
    metrics: dict = {}
    for label in ('service', 'gateway'):
        for cycle in range(sample_count(cfg, 'cycles', quick)):
            with policy_target(label, cfg) as (target, service):
                assert service.proc is not None
                transport = target.transport_factory()
                try:
                    for operation, entry in cfg['endpoints'].items():
                        (case,) = entry['cases']
                        prefix = f'{label}.{operation} [{case}]'
                        receipt, completion = measure(metrics, f'{prefix}.resources.{cycle}', Process(service.proc.pid),
                            partial(run_case, transport, target, service.home_dir, operation, cfg))
                        observations = Observations()
                        observations.append(receipt)
                        for key, value in observations.metrics(prefix + '.', 1).items():
                            metrics.setdefault(key, {'unit': value['unit'], 'samples': []})['samples'].extend(value['samples'])
                        metrics.setdefault(prefix + '.verified_host_completion_ms', {'unit': 'milliseconds', 'samples': []})['samples'].append(completion)
                finally:
                    transport.close()
    require_policy_metrics(metrics, cfg, quick)
    return metrics


def require_policy_metrics(metrics, cfg, quick):
    count = sample_count(cfg, 'cycles', quick)
    for label in ('service', 'gateway'):
        for operation, entry in cfg['endpoints'].items():
            (case,) = entry['cases']
            prefix = f'{label}.{operation} [{case}]'
            for suffix in ('latency_ms', 'headers_ms', 'first_body_byte_ms', 'verified_host_completion_ms'):
                key = prefix + '.' + suffix
                if key not in metrics or len(metrics[key]['samples']) != count:
                    raise ValueError(f'missing or partial policy metric: {key}')
            for cycle in range(count):
                key = f'{prefix}.resources.{cycle}.wall_s'
                if key not in metrics or len(metrics[key]['samples']) != 1:
                    raise ValueError('missing finite policy resource receipt')
