"""Source doubles exercise effect verification; no service or VM is launched."""
import json
from types import SimpleNamespace
from uuid import UUID

import pytest
from helpers.http_measurement import MeasuredResponse
from helpers.http_transport import Transport

from benchmarks.helpers import route_networks as subject
from benchmarks.helpers.route_requests import Target, json_request


class Backend:
    def __init__(self, root, failure=''):
        self.root, self.failure = root, failure
        self.calls, self.rows, self.counter = [], {}, 0

    def close(self):
        pass

    def measure_request(self, method, path, **kwargs):
        self.calls.append((method, path))
        status = 200
        if method == 'POST':
            self.counter += 1
            name = json.loads(kwargs['body'])['name']
            assert all(row['name'] != name for row in self.rows.values()), 'name was not released'
            ident = str(UUID(int=self.counter))
            value = {'id': ident, 'name': name, 'subnet': '10.14.0.0/24', 'created_unix_ms': 1, 'members': []}
            if self.failure != 'create-noop':
                self.rows[ident] = value.copy()
            if self.failure == 'public-subnet':
                value['subnet'] = '8.8.8.0/24'
            if self.failure != 'missing-file':
                file = self.root / 'networks' / ident / 'network.db'
                file.parent.mkdir(parents=True)
                file.write_bytes(b'synthetic fixture effect, not SQLite')
            status = 200 if self.failure == 'wrong-status' else 201
        elif method == 'DELETE':
            ident = path.split('/')[-1]
            assert ident in self.rows, 'deletion replay or unowned operation'
            if self.failure != 'delete-noop':
                del self.rows[ident]
            if self.failure == 'lost-audit':
                (self.root / 'networks' / ident / 'network.db').unlink()
            value = {'success': True}
        elif path == '/networks':
            value = {'networks': list(self.rows.values())}
        else:
            value = self.rows.get(path.split('/')[-1], {'error': 'not found'})
            status = 200 if 'id' in value else 404
        if self.failure == 'readback-wrong-name' and method == 'GET' and 'id' in value:
            value = {**value, 'name': 'wrong'}
        return MeasuredResponse(status, {}, json.dumps(value).encode(), 1, 2, 3,
            attempts=2 if self.failure == 'retry' else 1)


def setup(tmp_path, failure=''):
    backend = Backend(tmp_path, failure)
    cfg = subject.network_config()
    target = SimpleNamespace(label='service', headers={})
    return subject.Cycle(backend, target, SimpleNamespace(home_dir=tmp_path), cfg, {}), backend


def test_exact_created_status_is_supported_without_generic_2xx_acceptance():
    receipt = MeasuredResponse(201, {}, b'{"id":"synthetic"}', 1, 2, 3)
    transport = SimpleNamespace(measure_request=lambda *a, **kw: receipt)
    target = Target('service', lambda: Transport(host='127.0.0.1', port=1), {}, {})
    assert json_request(transport, target, 'POST', '/networks', {}, timeout=1, maximum=100, expected_status=201)[0] is receipt
    with pytest.raises(ValueError, match='201'):
        json_request(transport, target, 'POST', '/networks', {}, timeout=1, maximum=100)


def test_two_finite_mutations_require_independent_retirement_and_audit_effects(tmp_path):
    cycle, backend = setup(tmp_path)
    subject.sample_network(cycle)
    assert len(cycle.metrics) == 8 and backend.rows == {}
    assert len(list((tmp_path / 'networks').glob('*/network.db'))) == 2
    assert sum(method == 'POST' for method, _ in backend.calls) == 2  # second is untimed name-release probe
    deletes = [path for method, path in backend.calls if method == 'DELETE']
    assert len(deletes) == len(set(deletes)) == 2
    assert set(cycle.cfg['endpoints']) == {'POST /networks', 'DELETE /networks/{id}'}
    subject.require_network_metrics({**cycle.metrics, 'service.network_cycle.0.wall_s': {'samples': [1]}}, cycle.cfg, True, labels=('service',))


@pytest.mark.parametrize('failure', ['create-noop', 'delete-noop', 'missing-file', 'lost-audit', 'public-subnet', 'wrong-status', 'readback-wrong-name', 'retry'])
def test_echo_noop_partial_or_invalid_effects_do_not_publish_success(tmp_path, failure):
    cycle, backend = setup(tmp_path, failure)
    with pytest.raises((AssertionError, ValueError)):
        subject.sample_network(cycle)
    mutations = [call for call in backend.calls if call[0] != 'GET']
    assert len(mutations) == len(set(mutations)) or mutations.count(('POST', '/networks')) == 2
    if failure in {'create-noop', 'missing-file', 'public-subnet', 'wrong-status', 'readback-wrong-name', 'retry'}:
        assert cycle.metrics == {}


def test_deadline_and_unowned_refusal_happen_before_transport(monkeypatch, tmp_path):
    cycle, backend = setup(tmp_path)
    monkeypatch.setattr(subject.time, 'monotonic', lambda: 10)
    with pytest.raises(TimeoutError):
        cycle.call('POST', '/networks', {}, deadline=9, expected_status=201)
    with pytest.raises(AssertionError):
        cycle.retire(str(UUID(int=42)), deadline=11)
    assert backend.calls == []


def test_partial_counts_fail_and_config_bounds_cannot_disappear():
    from capsem_builder.gate.benchmarknetworkschema import NetworkRouteConfig
    cfg = subject.network_config()
    with pytest.raises(ValueError):
        NetworkRouteConfig.model_validate({**cfg, 'cycles': 100})
    with pytest.raises(ValueError):
        NetworkRouteConfig.model_validate({**cfg, 'endpoints': {}})
    with pytest.raises(ValueError, match='partial'):
        subject.require_network_metrics({}, cfg, True)


def test_completion_overrun_refuses_sample_even_after_success_receipt(monkeypatch, tmp_path):
    cycle, _ = setup(tmp_path)
    clocks = iter([0, 31])
    monkeypatch.setattr(subject.time, 'monotonic', lambda: next(clocks))
    receipt = MeasuredResponse(201, {}, b'{}', 1, 2, 3)
    monkeypatch.setattr(cycle, 'call', lambda *a, **kw: (receipt, {}))
    with pytest.raises(AssertionError, match='deadline'):
        cycle.observe('POST /networks', '/networks', {}, lambda *_: None, expected_status=201)
    assert cycle.metrics == {}


def test_collector_reuses_resource_owner_and_closes_every_transport(monkeypatch, tmp_path):
    from contextlib import contextmanager

    from benchmarks.helpers import route_network_fixture
    closed, measured = [], []
    @contextmanager
    def fixture(label):
        root = tmp_path / label
        root.mkdir()
        backend = Backend(root)
        monkeypatch.setattr(backend, 'close', lambda: closed.append(label))
        target = SimpleNamespace(label=label, headers={}, transport_factory=lambda: backend)
        yield target, SimpleNamespace(home_dir=root, proc=SimpleNamespace(pid=123))
    monkeypatch.setattr(route_network_fixture, 'network_target', fixture)
    monkeypatch.setattr(subject, 'Process', lambda pid: ('process', pid))
    def measure(metrics, phase, process, operation):
        assert process == ('process', 123)
        measured.append(phase)
        operation()
        metrics[phase + '.wall_s'] = {'unit': 'seconds', 'samples': [1]}
    monkeypatch.setattr(subject, 'measure', measure)
    metrics = subject.collect_networks(True)
    assert len(metrics) == 18 and closed == ['service', 'gateway']
    assert measured == ['service.network_cycle.0', 'gateway.network_cycle.0']
    key = next(key for key in metrics if key.endswith('first_body_byte_ms'))
    metrics[key]['samples'] *= 2
    with pytest.raises(ValueError, match='partial'):
        subject.require_network_metrics(metrics, subject.network_config(), True)


def test_fixture_preserves_existing_isolated_service_and_tears_down_failure(monkeypatch, tmp_path):
    import tomllib

    from benchmarks.helpers import route_network_fixture as fixture
    calls = []
    service = SimpleNamespace(home_dir=tmp_path, start=lambda: calls.append('start'), stop=lambda: calls.append('stop'))
    def create(**kwargs):
        assert kwargs == {'isolated_corp_config': True}
        return service
    monkeypatch.setattr(fixture, 'ServiceInstance', create)
    monkeypatch.setattr(fixture, 'clone_benchmark_assets', lambda *a: tmp_path / 'assets')
    monkeypatch.setattr(fixture, 'service_target', lambda owner, label: (owner, label))
    with pytest.raises(RuntimeError), fixture.network_target('gateway') as (target, owner):
        assert owner is service and target == (service, 'gateway')
        settings = tomllib.loads((tmp_path / 'settings.toml').read_text())
        assert settings['images']['catalog'] is False
        assert settings['settings']['app.auto_update']['value'] is False
        raise RuntimeError('synthetic failure')
    assert calls == ['start', 'stop']
