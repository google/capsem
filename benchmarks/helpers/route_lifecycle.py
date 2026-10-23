"""Finite owned lifecycle observations; admission and verified effects are separate."""
from __future__ import annotations

import shlex
import time
import tomllib
from functools import partial
from pathlib import Path
from types import SimpleNamespace
from uuid import UUID

from helpers.service import exec_output_text, vm_session_dir
from psutil import Process

from .route_measurements import Observations
from .route_requests import json_request
from .route_validation import SCHEMAS
from .stream_measurements import measure, sample_count

ROOT = Path(__file__).resolve().parents[2]
PROOF = '/workspace/capsem-route-lifecycle-proof'
PROBE = "printf 'CAPSEM-UID-MAP %s\\n' \"$(head -1 /proc/self/uid_map)\"; "


def lifecycle_config() -> dict:
    from capsem_builder.gate.benchmarklifecycleschema import LifecycleRouteConfig
    raw = tomllib.loads((ROOT / 'config/gate.toml').read_text())['benchmark']['routes']['lifecycle']
    return LifecycleRouteConfig.model_validate(raw).model_dump()


def validate_running(value, vm_id):
    assert value['id'] == vm_id and value['status'] == 'Running'
    assert value['persistent'] is True and type(value['pid']) is int and value['pid'] > 0


def validate_exec(value, expected):
    assert value['exit_code'] == 0 and value['truncated'] is False
    assert exec_output_text(value, 'stderr') == ''
    line, separator, output = exec_output_text(value).partition('\n')
    assert separator and line.startswith('CAPSEM-UID-MAP ')
    mapping = line.removeprefix('CAPSEM-UID-MAP ').split()
    assert len(mapping) == 3 and all(item.isdecimal() for item in mapping)
    assert tuple(mapping) != ('0', '0', '4294967295'), 'exec fell back to VM namespace'
    assert int(mapping[1]) > 0 and int(mapping[2]) > 0
    assert output == expected + '\n', 'workload bytes differ'


class Cycle:
    """One owned home, strict transport and exact known effects; never retries a mutation."""
    def __init__(self, transport, target, service, cfg, metrics):
        self.transport, self.target, self.service = transport, target, service
        self.cfg, self.metrics = cfg, metrics
        self.owned: set[str] = set()

    def call(self, method, path, body=None, *, deadline=None):
        remaining = self.cfg['timeout_seconds'] if deadline is None else deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('lifecycle completion deadline exceeded')
        receipt, value = json_request(self.transport, self.target, method, path, body,
            timeout=remaining, maximum=self.cfg['max_response_bytes'])
        parts = path.split('/')
        schema_path = '/vms/{id}/' + parts[3] if len(parts) == 4 and parts[1] == 'vms' else path
        if schema_path in SCHEMAS:
            SCHEMAS[schema_path].validate_json(receipt.body)
        return receipt, value

    def read(self, path, *, deadline=None):
        return self.call('GET', path, deadline=deadline)[1]

    def listing(self, *, deadline=None):
        return self.read('/vms/list', deadline=deadline)['sandboxes']

    def identify(self, value):
        vm_id = value['id']
        assert str(UUID(vm_id)) == vm_id and vm_id not in self.owned
        self.owned.add(vm_id)
        return vm_id

    def info(self, vm_id, *, deadline=None):
        assert vm_id in self.owned, 'refusing unowned VM'
        return self.read(f'/vms/{vm_id}/info', deadline=deadline)

    def session(self, vm_id, *, deadline=None):
        client = SimpleNamespace(get=lambda path: self.read(path, deadline=deadline))
        path = vm_session_dir(self.service.tmp_dir, client, vm_id)
        assert path.resolve().is_relative_to(self.service.tmp_dir.resolve())
        return path

    def execute(self, vm_id, command, *, deadline=None):
        assert vm_id in self.owned
        return self.call('POST', f'/vms/{vm_id}/exec', {'command': PROBE + command,
            'timeout_secs': self.cfg['exec_timeout_seconds'], 'target': 'workload'}, deadline=deadline)

    def ready(self, vm_id, reference, *, deadline):
        while True:
            info = self.info(vm_id, deadline=deadline)
            validate_running(info, vm_id)
            container = self.read(f'/vms/{vm_id}/container', deadline=deadline)
            assert container['state'] not in {'failed', 'exited'}, container
            if container['state'] == 'running':
                assert container['image'] == reference and container['digest'] == reference.split('@')[1]
                _, result = self.execute(vm_id, "printf 'ready\\n'", deadline=deadline)
                validate_exec(result, 'ready')
                return info
            time.sleep(min(self.cfg['poll_seconds'], max(0, deadline - time.monotonic())))

    def observe(self, operation, path, body, verify):
        method = operation.split(' ', 1)[0]
        started = time.monotonic_ns()
        deadline = time.monotonic() + self.cfg['timeout_seconds']
        receipt, result = self.call(method, path, body, deadline=deadline)
        verified = verify(result, deadline)
        elapsed = (time.monotonic_ns() - started) / 1_000_000
        assert time.monotonic() <= deadline, 'lifecycle verification exceeded deadline'
        (case,) = self.cfg['endpoints'][operation]['cases']
        prefix = f'{self.target.label}.{operation} [{case}]'
        observations = Observations()
        observations.append(receipt)
        for key, value in observations.metrics(prefix + '.', 1).items():
            self.metrics.setdefault(key, {'unit': value['unit'], 'samples': []})['samples'].extend(value['samples'])
        self.metrics.setdefault(prefix + '.verified_completion_ms', {'unit': 'milliseconds', 'samples': []})['samples'].append(elapsed)
        return verified

    def absent(self, vm_id, path, process, *, deadline=None):
        assert not any(row['id'] == vm_id for row in self.listing(deadline=deadline))
        assert not path.exists() and not process.is_running(), 'deleted owner retained path/process'
        self.owned.remove(vm_id)

    def cleanup(self):
        # Original mutations are never replayed on failure. The service fixture's
        # normal teardown owns emergency cleanup if an acknowledgement was lost.
        for vm_id in tuple(self.owned):
            result = self.call('DELETE', f'/vms/{vm_id}/delete')[1]
            assert result == {'success': True}
            self.owned.remove(vm_id)


def sample_lifecycle(cycle, reference, certificate):
    assert cycle.listing() == [], 'lifecycle fixture already owns sessions'
    cfg = cycle.cfg
    def created(value, deadline):
        vm_id = cycle.identify(value)
        info = cycle.ready(vm_id, reference, deadline=deadline)
        assert info['name'] == 'route-source'
        cycle.session(vm_id, deadline=deadline)
        return vm_id
    source = cycle.observe('POST /vms/create', '/vms/create', {'name': 'route-source', 'persistent': True,
        'ram_mb': cfg['ram_mb'], 'cpus': cfg['cpus'], 'container': {'image': reference,
        'registry': {'ca_pem': certificate.read_text()}}}, created)
    source_pid = cycle.info(source)['pid']
    quoted = shlex.quote(cfg['marker'])
    def executed(value, deadline):
        validate_exec(value, cfg['marker'])
        _, readback = cycle.execute(source, f'cat {PROOF}', deadline=deadline)
        validate_exec(readback, cfg['marker'])
    cycle.observe('POST /vms/{id}/exec', f'/vms/{source}/exec', {'command': PROBE +
        f"set -eu; printf '%s\\n' {quoted} > {PROOF}; cat {PROOF}",
        'target': 'workload', 'timeout_secs': cfg['exec_timeout_seconds']}, executed)
    def forked(value, deadline):
        fork = cycle.identify(value)
        info = cycle.info(fork, deadline=deadline)
        assert info['status'] == 'Stopped' and info['persistent'] is True
        assert info['name'] == value['name'] == 'route-fork' and info['can_resume'] is True
        assert type(value['size_bytes']) is int and value['size_bytes'] > 0
        cycle.session(fork, deadline=deadline)
        original = cycle.info(source, deadline=deadline)
        validate_running(original, source)
        assert original['pid'] == source_pid
        return fork
    fork = cycle.observe('POST /vms/{id}/fork', f'/vms/{source}/fork', {'name': 'route-fork'}, forked)
    def started(value, deadline):
        assert value['id'] == fork and value['persistent'] is True
        info = cycle.ready(fork, reference, deadline=deadline)
        _, result = cycle.execute(fork, f'cat {PROOF}', deadline=deadline)
        validate_exec(result, cfg['marker'])
        return info
    running = cycle.observe('POST /vms/{id}/start', f'/vms/{fork}/start', {}, started)
    process = Process(running['pid'])  # psutil retains original process identity against PID reuse.
    path = cycle.session(fork)
    def stopped(value, deadline):
        assert value == {'success': True, 'persistent': True}
        assert cycle.info(fork, deadline=deadline)['status'] == 'Stopped'
        assert not process.is_running() and path.is_dir(), 'stop did not exit/retain owner'
    cycle.observe('POST /vms/{id}/stop', f'/vms/{fork}/stop', {}, stopped)
    def deleted(value, deadline):
        assert value == {'success': True}
        cycle.absent(fork, path, process, deadline=deadline)
    cycle.observe('DELETE /vms/{id}/delete', f'/vms/{fork}/delete', None, deleted)
    assert {row['id'] for row in cycle.listing()} == {source}


def collect_lifecycle(quick):
    from .route_lifecycle_fixture import lifecycle_target
    cfg, metrics = lifecycle_config(), {}
    for label in ('service', 'gateway'):
        for index in range(sample_count(cfg, 'cycles', quick)):
            with lifecycle_target(label) as (target, service, reference, certificate):
                transport = target.transport_factory()
                cycle = Cycle(transport, target, service, cfg, metrics)
                try:
                    assert service.proc is not None
                    measure(metrics, f'{label}.lifecycle_cycle.{index}', Process(service.proc.pid),
                        partial(sample_lifecycle, cycle, reference, certificate))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_lifecycle_metrics(metrics, cfg, quick)
    return metrics


def require_lifecycle_metrics(metrics, cfg, quick):
    count = sample_count(cfg, 'cycles', quick)
    for label in ('service', 'gateway'):
        for operation, entry in cfg['endpoints'].items():
            (case,) = entry['cases']
            for suffix in ('latency_ms', 'headers_ms', 'first_body_byte_ms', 'verified_completion_ms'):
                key = f'{label}.{operation} [{case}].{suffix}'
                if key not in metrics or len(metrics[key]['samples']) != count:
                    raise ValueError(f'missing or partial lifecycle metric: {key}')
        for index in range(count):
            key = f'{label}.lifecycle_cycle.{index}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing or partial lifecycle resources')
