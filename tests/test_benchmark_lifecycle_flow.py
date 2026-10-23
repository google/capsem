"""Synthetic lifecycle effects and failures; no product process or VM runs."""
from __future__ import annotations

import json
from contextlib import contextmanager
from types import SimpleNamespace
from uuid import UUID

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_lifecycle as subject
from benchmarks.helpers import route_lifecycle_fixture

REFERENCE = '127.0.0.1:1234/library/dev@sha256:' + 'a' * 64


class Backend:
    def __init__(self, root, cfg):
        self.root, self.cfg = root, cfg
        self.rows, self.markers, self.alive, self.calls = {}, {}, {}, []
        self.failure = ''
        self.closed = False

    def close(self):
        self.closed = True

    def process(self, pid):
        # A captured identity; deleted/replaced process cannot be mistaken for the old owner.
        return SimpleNamespace(is_running=lambda: self.alive.get(pid, False))

    def measure_request(self, method, path, *, body=None, **limits):
        assert limits['timeout'] > 0 and limits['max_response_bytes'] == self.cfg['max_response_bytes']
        self.calls.append((method, path))
        parts = path.strip('/').split('/')
        vm_id = parts[1] if len(parts) > 2 else None
        data = json.loads(body) if body is not None else {}
        if method == 'POST':
            assert body is not None and isinstance(data, dict)
        if path == '/vms/list':
            value = {'sandboxes': list(self.rows.values())}
        elif path == '/vms/create':
            assert data['persistent'] is True and data['container']['image'] == REFERENCE
            assert data['ram_mb'] == self.cfg['ram_mb'] and data['cpus'] == self.cfg['cpus']
            value = self.add(1, 'route-source', 'Running')
        elif path.endswith('/info'):
            value = self.rows[vm_id]
        elif path.endswith('/container'):
            value = {'state': 'running', 'image': REFERENCE, 'digest': REFERENCE.split('@')[1]}
            if self.failure == 'wrong-image':
                value['image'] = 'other'
        elif path.endswith('/exec'):
            assert data['target'] == 'workload' and data['command'].startswith(subject.PROBE)
            if '> ' + subject.PROOF in data['command']:
                if self.failure != 'write-noop':
                    self.markers[vm_id] = self.cfg['marker']
                output = self.cfg['marker']
            elif 'cat ' + subject.PROOF in data['command']:
                output = self.markers.get(vm_id, 'missing')
            else:
                output = 'ready'
            mapping = '0 0 4294967295' if self.failure == 'vm-fallback' else '0 100000 65536'
            value = {'exit_code': 0, 'truncated': False,
                'stdout': {'encoding': 'utf8', 'data': f'CAPSEM-UID-MAP {mapping}\n{output}\n'},
                'stderr': {'encoding': 'utf8', 'data': ''}}
        elif path.endswith('/fork'):
            fork = self.add(2, 'route-fork', 'Running' if self.failure == 'fork-running' else 'Stopped')
            self.markers[fork['id']] = self.markers[vm_id] if self.failure != 'lost-fork-bytes' else 'lost'
            if self.failure == 'changed-source':
                self.rows[vm_id]['pid'] += 10
            value = {'id': fork['id'], 'name': fork['name'], 'size_bytes': 40}
        elif path.endswith('/start'):
            value = self.rows[vm_id]
            value.update(status='Running', pid=52)
            self.alive[52] = True
        elif path.endswith('/stop'):
            row = self.rows[vm_id]
            if self.failure != 'live-stop':
                self.alive[row['pid']] = False
            row['status'] = 'Stopped'
            value = {'success': True, 'persistent': True}
        elif path.endswith('/delete'):
            row = self.rows.pop(vm_id)
            self.alive[row['pid']] = False
            if self.failure != 'retained-delete-path':
                (self.root / 'persistent' / vm_id).rmdir()
            value = {'success': True}
        else:
            raise AssertionError((method, path))
        status = 403 if self.failure == 'refusal' and method == 'POST' else 200
        return MeasuredResponse(status, {'Content-Type': 'application/json'}, json.dumps(value).encode(), 1, 2, 3)

    def add(self, index, name, status):
        vm_id = str(UUID(int=index))
        pid = 50 + index if status == 'Running' else 0
        self.rows[vm_id] = {'id': vm_id, 'name': name, 'persistent': True,
            'status': status, 'pid': pid, 'can_resume': True, 'available_actions': []}
        self.alive[pid] = status == 'Running'
        (self.root / 'persistent' / vm_id).mkdir(parents=True)
        return self.rows[vm_id]


def setup(monkeypatch, tmp_path):
    cfg = subject.lifecycle_config()
    backend = Backend(tmp_path, cfg)
    target = SimpleNamespace(label='service', headers={})
    cycle = subject.Cycle(backend, target, SimpleNamespace(tmp_dir=tmp_path), cfg, {})
    certificate = tmp_path / 'certificate'
    certificate.write_text('Synthetic certificate; no TLS server is run')
    monkeypatch.setattr(subject, 'Process', backend.process)
    return cycle, backend, certificate


def test_six_finite_mutations_verify_real_synthetic_effects_separately(monkeypatch, tmp_path):
    cycle, backend, certificate = setup(monkeypatch, tmp_path)
    subject.sample_lifecycle(cycle, REFERENCE, certificate)
    assert len(cycle.metrics) == 24
    assert {key.split(' [')[0].removeprefix('service.') for key in cycle.metrics} == set(cycle.cfg['endpoints'])
    mutations = [(method, path) for method, path in backend.calls if method != 'GET']
    source, fork = str(UUID(int=1)), str(UUID(int=2))
    for operation, path in [('POST', '/vms/create'), ('POST', f'/vms/{source}/fork'),
        ('POST', f'/vms/{fork}/start'), ('POST', f'/vms/{fork}/stop'), ('DELETE', f'/vms/{fork}/delete')]:
        assert mutations.count((operation, path)) == 1
    assert mutations.count(('POST', f'/vms/{source}/exec')) == 3  # ready, stimulus, independent readback
    assert set(backend.rows) == {source}
    cycle.cleanup()
    assert backend.rows == {} and not cycle.owned


@pytest.mark.parametrize('failure', ['write-noop', 'vm-fallback', 'fork-running', 'lost-fork-bytes',
    'changed-source', 'live-stop', 'retained-delete-path', 'wrong-image', 'refusal'])
def test_admission_or_echo_cannot_certify_completion(monkeypatch, tmp_path, failure):
    cycle, backend, certificate = setup(monkeypatch, tmp_path)
    backend.failure = failure
    with pytest.raises((AssertionError, ValueError)):
        subject.sample_lifecycle(cycle, REFERENCE, certificate)
    assert backend.calls.count(('POST', '/vms/create')) == 1
    # No retry/replayed mutation and no automatic repeat of a lost acknowledgement.
    mutations = [(method, path) for method, path in backend.calls if method != 'GET' and not path.endswith('/exec')]
    assert len(mutations) == len(set(mutations))


def test_one_total_completion_deadline_no_mutation_retry(monkeypatch, tmp_path):
    cycle, backend, _ = setup(monkeypatch, tmp_path)
    monkeypatch.setattr(subject.time, 'monotonic', lambda: 10)
    with pytest.raises(TimeoutError):
        cycle.call('POST', '/vms/create', {}, deadline=9)
    assert backend.calls == []


def test_collect_uses_existing_resource_owner_and_requires_complete_counts(monkeypatch, tmp_path):
    cfg = subject.lifecycle_config()
    monkeypatch.setattr(subject, 'lifecycle_config', lambda: cfg)
    backends = []
    @contextmanager
    def fixture(label):
        root = tmp_path / label
        root.mkdir()
        backend = Backend(root, cfg)
        backends.append(backend)
        certificate = root / 'certificate'
        certificate.write_text('synthetic')
        target = SimpleNamespace(label=label, headers={}, transport_factory=lambda: backend)
        monkeypatch.setattr(subject, 'Process', backend.process)
        yield target, SimpleNamespace(tmp_dir=root, proc=SimpleNamespace(pid=77)), REFERENCE, certificate
    monkeypatch.setattr(route_lifecycle_fixture, 'lifecycle_target', fixture)
    def measure(metrics, phase, process, operation):
        assert process is not None
        result = operation()
        metrics[phase + '.wall_s'] = {'unit': 'seconds', 'samples': [1.0]}
        return result
    monkeypatch.setattr(subject, 'measure', measure)
    metrics = subject.collect_lifecycle(True)
    assert len(metrics) == 50 and all(backend.closed and not backend.rows for backend in backends)
    subject.require_lifecycle_metrics(metrics, cfg, True)
    metrics.pop(next(key for key in metrics if key.endswith('first_body_byte_ms')))
    with pytest.raises(ValueError, match='missing or partial'):
        subject.require_lifecycle_metrics(metrics, cfg, True)


def test_unowned_control_refused_before_transport(monkeypatch, tmp_path):
    cycle, backend, _ = setup(monkeypatch, tmp_path)
    with pytest.raises(AssertionError, match='unowned'):
        cycle.info(str(UUID(int=88)))
    assert backend.calls == []


def test_completion_deadline_failure_does_not_publish_a_sample(monkeypatch, tmp_path):
    cycle, _, _ = setup(monkeypatch, tmp_path)
    cycle.cfg = {**cycle.cfg, 'timeout_seconds': 1}
    clocks = iter([0, 2])
    monkeypatch.setattr(subject.time, 'monotonic', lambda: next(clocks))
    receipt = MeasuredResponse(200, {}, b'{}', 1, 2, 3)
    monkeypatch.setattr(cycle, 'call', lambda *_, **__: (receipt, {}))
    with pytest.raises(AssertionError, match='deadline'):
        cycle.observe('POST /vms/create', '/vms/create', {}, lambda *_: None)
    assert cycle.metrics == {}
