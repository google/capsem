"""Finite owned empty-network lifecycle; no member/transport or crash claims."""
from __future__ import annotations

import time
import tomllib
from functools import partial
from ipaddress import IPv4Network
from pathlib import Path
from uuid import UUID

from psutil import Process

from .route_measurements import Observations
from .route_requests import json_request
from .route_validation import SCHEMAS
from .stream_measurements import measure, sample_count

ROOT = Path(__file__).resolve().parents[2]


def network_config():
    from capsem_builder.gate.benchmarknetworkschema import NetworkRouteConfig
    raw = tomllib.loads((ROOT / 'config/gate.toml').read_text())['benchmark']['routes']['networks']
    return NetworkRouteConfig.model_validate(raw).model_dump()


class Cycle:
    """One private service generation and one-shot control receipts."""
    def __init__(self, transport, target, service, cfg, metrics):
        self.transport, self.target, self.service = transport, target, service
        self.cfg, self.metrics = cfg, metrics
        self.owned: set[str] = set()

    def call(self, method, path, body=None, *, deadline, expected_status=200):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('network completion deadline exceeded')
        receipt, result = json_request(self.transport, self.target, method, path, body,
            timeout=remaining, maximum=self.cfg['max_response_bytes'], expected_status=expected_status)
        if expected_status in {200, 201} and isinstance(result, dict) and 'id' in result:
            SCHEMAS['/networks/{id}'].validate_json(receipt.body)
        return receipt, result

    def read(self, path, *, deadline, expected_status=200):
        return self.call('GET', path, deadline=deadline, expected_status=expected_status)[1]

    def listing(self, *, deadline):
        value = self.read('/networks', deadline=deadline)
        assert set(value) == {'networks'} and isinstance(value['networks'], list)
        return value['networks']

    def database(self, ident):
        assert ident in self.owned and str(UUID(ident)) == ident, 'unowned network'
        directory = self.service.home_dir / 'networks' / ident
        path = directory / 'network.db'
        assert not directory.is_symlink() and not path.is_symlink()
        assert path.is_file(), 'owned network database was not materialized/retained'
        return path

    def created(self, value, *, deadline):
        ident = value['id']
        assert str(UUID(ident)) == ident and ident not in self.owned, 'network identity reused'
        assert value['name'] == self.cfg['name'] and value['members'] == []
        assert type(value['created_unix_ms']) is int and value['created_unix_ms'] > 0
        network = IPv4Network(value['subnet'])
        assert network.is_private and network.prefixlen == 24, 'unexpected network subnet'
        self.owned.add(ident)
        assert self.listing(deadline=deadline) == [value], 'create did not publish exact independent list'
        assert self.read(f'/networks/{ident}', deadline=deadline) == value, 'network detail differs'
        self.database(ident)
        return ident

    def retire(self, ident, *, deadline):
        assert ident in self.owned, 'unowned network'
        return self.call('DELETE', f'/networks/{ident}', deadline=deadline)

    def absent(self, ident, value, *, deadline):
        assert value == {'success': True}
        assert self.listing(deadline=deadline) == [], 'retired network remains active'
        self.read(f'/networks/{ident}', deadline=deadline, expected_status=404)
        self.database(ident)  # retirement retains audit history; not a SQLite read

    def observe(self, operation, path, body, verify, *, expected_status=200):
        started = time.monotonic_ns()
        deadline = time.monotonic() + self.cfg['timeout_seconds']
        receipt, value = self.call(operation.split(' ', 1)[0], path, body,
            deadline=deadline, expected_status=expected_status)
        verified = verify(value, deadline)
        elapsed = (time.monotonic_ns() - started) / 1_000_000
        assert time.monotonic() <= deadline, 'network verification exceeded deadline'
        (case,) = self.cfg['endpoints'][operation]['cases']
        prefix = f'{self.target.label}.{operation} [{case}]'
        observations = Observations()
        observations.append(receipt)
        for key, metric in observations.metrics(prefix + '.', 1).items():
            self.metrics.setdefault(key, {'unit': metric['unit'], 'samples': []})['samples'].extend(metric['samples'])
        self.metrics.setdefault(prefix + '.verified_completion_ms', {'unit': 'milliseconds', 'samples': []})['samples'].append(elapsed)
        return verified


def sample_network(cycle):
    deadline = time.monotonic() + cycle.cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == [], 'network fixture is not isolated/empty'
    ident = cycle.observe('POST /networks', '/networks', {'name': cycle.cfg['name']},
        lambda value, deadline: cycle.created(value, deadline=deadline), expected_status=201)
    def retired(value, deadline):
        cycle.absent(ident, value, deadline=deadline)
        # Independent real name-release effect. This second create/delete is
        # untimed control work inside verified completion, never a replay of
        # the measured delete. Both audit databases stay with the private home.
        _, replacement = cycle.call('POST', '/networks', {'name': cycle.cfg['name']},
            deadline=deadline, expected_status=201)
        second = cycle.created(replacement, deadline=deadline)
        assert second != ident
        _, removed = cycle.retire(second, deadline=deadline)
        cycle.absent(second, removed, deadline=deadline)
        cycle.owned.remove(second)
        cycle.owned.remove(ident)
    cycle.observe('DELETE /networks/{id}', f'/networks/{ident}', None, retired)
    assert not cycle.owned


def collect_networks(quick):
    from .route_network_fixture import network_target
    cfg, metrics = network_config(), {}
    for label in ('service', 'gateway'):
        for index in range(sample_count(cfg, 'cycles', quick)):
            with network_target(label) as (target, service):
                transport = target.transport_factory()
                try:
                    assert service.proc is not None
                    cycle = Cycle(transport, target, service, cfg, metrics)
                    # Resources cover the whole finite cycle including probes;
                    # they are not attributed as per-request CPU measurements.
                    measure(metrics, f'{label}.network_cycle.{index}', Process(service.proc.pid),
                        partial(sample_network, cycle))
                finally:
                    transport.close()
    require_network_metrics(metrics, cfg, quick)
    return metrics


def require_network_metrics(metrics, cfg, quick, labels=('service', 'gateway')):
    count = sample_count(cfg, 'cycles', quick)
    for label in labels:
        for operation, entry in cfg['endpoints'].items():
            (case,) = entry['cases']
            for suffix in ('latency_ms', 'headers_ms', 'first_body_byte_ms', 'verified_completion_ms'):
                key = f'{label}.{operation} [{case}].{suffix}'
                if key not in metrics or len(metrics[key]['samples']) != count:
                    raise ValueError(f'missing or partial network metric: {key}')
        for index in range(count):
            key = f'{label}.network_cycle.{index}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing or partial network resource sample')
