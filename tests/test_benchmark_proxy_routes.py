"""Explicit source doubles, never a native proxy or performance run."""
import json
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_proxies as subject


class Probe:
    def __init__(self, failure=''):
        self.now, self.rows, self.calls = 0., {}, []
        self.failure = failure
        self.target = SimpleNamespace(label='service')
        self.service = SimpleNamespace(proc=SimpleNamespace(pid=123))

    def clock(self):
        return self.now

    def wait(self, until, deadline):
        assert until <= deadline
        self.now = until
        for row in list(self.rows.values()):
            if row['expiry'] <= self.now and self.failure != 'no-expiry':
                row['alive'] = False

    def call(self, method, path, body, deadline):
        assert deadline > self.now
        self.calls.append(path)
        if path == '/proxies':
            ident = 'proxy-' + f'{len(self.rows)+1:032x}'
            value = {'session_id': ident, 'provider': 'openai', 'bind': '127.0.0.1', 'port': 1234,
                'base_url': 'http://127.0.0.1:1234/v1', 'lease_token': 'private-token-'+ident,
                'lease_expires_unix_ms': int((self.now+15)*1000)}
            self.rows[ident] = {'value': value, 'expiry': self.now+15, 'alive': True, 'identity': ident}
        else:
            ident = path.split('/')[2]
            row = self.rows[ident]
            assert body == {'lease_token': row['value']['lease_token']}
            if path.endswith('/heartbeat'):
                if self.failure != 'echo-renewal':
                    row['expiry'] = self.now+15
                if self.failure == 'replace-worker':
                    row['identity'] = 'new-owner'
                value = {'session_id': ident, 'lease_expires_unix_ms': int((self.now+15)*1000)}
            else:
                if self.failure != 'stop-noop':
                    row['alive'] = False
                value = {'session_id': ident, 'stopped': True}
        return MeasuredResponse(200, {}, json.dumps(value).encode(), 1, 2, 3,
            attempts=2 if self.failure == 'retry' else 1), value

    def capture(self, value):
        return {'value': value, 'identity': self.rows[value['session_id']]['identity']}

    def model(self, owner, deadline):
        row = self.rows[owner['value']['session_id']]
        assert row['alive'], 'listener expired despite renewal'
        assert row['identity'] == owner['identity'], 'worker identity changed'
        assert self.failure != 'wrong-model', 'wrong model response/upstream fact'

    def settled(self, owner, deadline):
        assert not self.rows[owner['value']['session_id']]['alive'], 'listener/process/session not settled'


def cycle(failure=''):
    probe = Probe(failure)
    return subject.Cycle(probe, subject.proxy_config(), {}), probe


def test_original_renewal_survives_expiry_control_and_stop_settles():
    run, probe = cycle()
    subject.sample_proxy(run)
    assert len(run.metrics) == 12 and all(not row['alive'] for row in probe.rows.values())
    assert probe.now == 16
    assert probe.calls.count('/proxies') == 2
    assert len([p for p in probe.calls if p.endswith('/heartbeat')]) == 1
    assert len([p for p in probe.calls if p.endswith('/stop')]) == 1
    assert 'private-token' not in json.dumps(run.metrics)
    subject.require_proxy_metrics(run.metrics, run.cfg, True, labels=('service',))


@pytest.mark.parametrize('failure', ['echo-renewal','replace-worker','stop-noop','no-expiry','wrong-model','retry'])
def test_echo_replacement_partial_and_unsettled_effects_are_refused(failure):
    run, _ = cycle(failure)
    with pytest.raises((AssertionError, ValueError)):
        subject.sample_proxy(run)


def test_missing_and_duplicate_samples_fail():
    run, _ = cycle()
    with pytest.raises(ValueError, match='partial'):
        subject.require_proxy_metrics({}, run.cfg, True)
    subject.sample_proxy(run)
    key = next(iter(run.metrics))
    run.metrics[key]['samples'] *= 2
    with pytest.raises(ValueError, match='partial'):
        subject.require_proxy_metrics(run.metrics, run.cfg, True, labels=('service',))


def test_config_preserves_real_ttl_and_finite_observation_budget():
    from capsem_builder.gate.benchmarkproxyschema import ProxyRouteConfig
    cfg = subject.proxy_config()
    for change in ({'lease_ttl_seconds':1}, {'renew_after_seconds':15}, {'timeout_seconds':10}, {'cycles':100}, {'endpoints':{}}):
        with pytest.raises(ValueError):
            ProxyRouteConfig.model_validate({**cfg, **change})


def test_receipt_and_end_to_end_deadline_refused_before_metrics():
    run, probe = cycle()
    receipt = MeasuredResponse(200, {}, b'{}', 1, 2, 3)
    def operation():
        probe.now = 100
        return receipt, {}
    with pytest.raises(AssertionError, match='deadline'):
        run.observe('POST /proxies', operation, lambda value: value, deadline=10)
    assert run.metrics == {}


def test_collector_keeps_existing_process_tree_owner_and_exact_counts(monkeypatch):
    from contextlib import contextmanager

    from benchmarks.helpers import route_proxy_fixture
    measured, closed = [], []
    @contextmanager
    def fixture(label,cfg):
        probe = Probe()
        probe.target.label = label
        probe.service = SimpleNamespace(proc=SimpleNamespace(pid=123))
        try:
            yield probe
        finally:
            closed.append(label)
    monkeypatch.setattr(route_proxy_fixture,'proxy_target',fixture)
    monkeypatch.setattr(subject,'Process',lambda pid:('process',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('process',123)
        measured.append(phase)
        operation()
        metrics[phase+'.wall_s'] = {'unit':'seconds','samples':[1]}
    monkeypatch.setattr(subject,'measure',measure)
    metrics = subject.collect_proxies(True)
    assert len(metrics) == 26 and closed == ['service','gateway']
    assert measured == ['service.proxy_cycle.0','gateway.proxy_cycle.0']
