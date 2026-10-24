"""Finite proxy lease effects; observations contain no private control values."""
import tomllib
from functools import partial
from pathlib import Path

from psutil import Process

from .route_measurements import Observations
from .stream_measurements import measure, sample_count


def proxy_config():
    from capsem_builder.gate.benchmarkproxyschema import ProxyRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2] / 'config/gate.toml').read_text())
    return ProxyRouteConfig.model_validate(raw['benchmark']['routes']['proxies']).model_dump()


class Cycle:
    def __init__(self, probe, cfg, metrics):
        self.probe, self.cfg, self.metrics = probe, cfg, metrics

    def observe(self, operation, request, verify, *, deadline):
        started = self.probe.clock()
        receipt, value = request()
        result = verify(value)
        assert self.probe.clock() <= deadline, 'proxy verification deadline exceeded'
        observations = Observations()
        observations.append(receipt)
        (case,) = self.cfg['endpoints'][operation]['cases']
        prefix = f'{self.probe.target.label}.{operation} [{case}].'
        for key, metric in observations.metrics(prefix, 1).items():
            self.metrics.setdefault(key, {'unit':metric['unit'], 'samples':[]})['samples'].extend(metric['samples'])
        self.metrics.setdefault(prefix+'verified_completion_ms', {'unit':'milliseconds','samples':[]})['samples'].append((self.probe.clock()-started)*1000)
        return result


def sample_proxy(cycle):
    p, cfg = cycle.probe, cycle.cfg
    deadline = p.clock() + cfg['timeout_seconds']
    request = {'provider':cfg['provider'], 'bind':'127.0.0.1', 'port':0}
    # Separate unrenewed control; setup is never a measured create sample.
    _, value = p.call('POST', '/proxies', request, deadline)
    control = p.capture(value)
    p.model(control, deadline)
    def created(value):
        owner = p.capture(value)
        p.model(owner, deadline)
        return owner
    owner = cycle.observe('POST /proxies', lambda: p.call('POST','/proxies',request,deadline), created, deadline=deadline)
    born = p.clock()  # after ready: TTL began earlier, never guess from request start
    ident, token = owner['value']['session_id'], owner['value']['lease_token']
    p.wait(born+cfg['renew_after_seconds'], deadline)
    def renewed(value):
        assert value['session_id'] == ident and value['lease_expires_unix_ms'] > owner['value']['lease_expires_unix_ms'], 'lease response identity/expiry invalid'
        p.wait(born+cfg['lease_ttl_seconds']+cfg['expiry_margin_seconds'], deadline)
        p.settled(control, deadline)
        p.model(owner, deadline)  # original worker/listener survives original TTL
    cycle.observe('POST /proxies/{id}/heartbeat',
        lambda: p.call('POST',f'/proxies/{ident}/heartbeat',{'lease_token':token},deadline), renewed, deadline=deadline)
    def stopped(value):
        assert value == {'session_id':ident, 'stopped':True}, 'proxy stop acknowledgement invalid'
        p.settled(owner, deadline)
    cycle.observe('POST /proxies/{id}/stop',
        lambda: p.call('POST',f'/proxies/{ident}/stop',{'lease_token':token},deadline), stopped, deadline=deadline)


def require_proxy_metrics(metrics, cfg, quick, labels=('service','gateway')):
    count = sample_count(cfg, 'cycles', quick)
    for label in labels:
        for operation, entry in cfg['endpoints'].items():
            (case,) = entry['cases']
            for suffix in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                key = f'{label}.{operation} [{case}].{suffix}'
                if key not in metrics or len(metrics[key]['samples']) != count:
                    raise ValueError('missing or partial proxy lifecycle samples')


def collect_proxies(quick):
    from .route_proxy_fixture import proxy_target
    cfg, metrics = proxy_config(), {}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with proxy_target(label,cfg) as probe:
                assert probe.service.proc is not None
                measure(metrics,f'{label}.proxy_cycle.{index}',Process(probe.service.proc.pid),
                    partial(sample_proxy,Cycle(probe,cfg,metrics)))
    require_proxy_metrics(metrics,cfg,quick)
    return metrics
