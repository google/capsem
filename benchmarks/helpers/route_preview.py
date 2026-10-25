"""Finite preview control receipts; port wiring is not Xpra/render proof."""
import time
import tomllib
from functools import partial
from pathlib import Path
from urllib.parse import urlencode, urlsplit

from helpers.http_transport import Transport
from psutil import Process

from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count

CREATE = 'POST /vms/{id}/exposures'
DELETE = 'DELETE /vms/{id}/exposures/{exposure_id}'
MINT = 'POST /vms/{id}/exposures/{exposure_id}/preview-session'
REVOKE = 'DELETE /vms/{id}/exposures/{exposure_id}/preview-session'
SURFACE = 'POST /vms/{id}/surface/session'


def preview_config():
    from capsem_builder.gate.benchmarkpreviewschema import PreviewRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return PreviewRouteConfig.model_validate(raw['benchmark']['routes']['preview_controls']).model_dump()


def material(value,exposure,port,cfg):
    assert set(value) == {'url','bootstrap_token','expires_in_seconds'}
    url = urlsplit(value['url'])
    assert value['url'] == f'http://{exposure}.localhost:{port}/_capsem/bootstrap', 'foreign capability origin'
    assert url.scheme == 'http' and not url.query and not url.fragment and not url.username
    token = value['bootstrap_token']
    assert isinstance(token,str) and 0 < len(token.encode()) <= cfg['max_token_bytes']
    assert type(value['expires_in_seconds']) is int and 0 < value['expires_in_seconds'] <= cfg['bootstrap_lifetime_seconds']
    return url.path,urlencode({'bootstrap_token':token}).encode()


def bootstrap_cookie(receipt,surface):
    # Empty 303 has no body observation. Do not invent a first-byte sample.
    assert receipt.attempts == 1 and receipt.status == 303 and receipt.body == b''
    assert receipt.first_body_byte_ns is None and 0 <= receipt.headers_ns <= receipt.complete_ns
    headers = receipt.headers
    assert headers['location'] == ('/_capsem/surface/' if surface else '/')
    assert headers['cache-control'] == 'no-store'
    parts = headers['set-cookie'].split('; ')
    cookie = parts[0]
    assert cookie.startswith('capsem_preview=') and len(cookie) > len('capsem_preview=')
    assert {'Path=/','HttpOnly','SameSite=Lax','Max-Age=900'} <= set(parts[1:])
    return cookie


def publication_generation(value):
    """Public listener identity is a decimal-string nonzero u64, not UUID."""
    assert isinstance(value,str) and 1 <= len(value) <= 20 and value.isascii() and value.isdecimal()
    integer = int(value)
    assert 0 < integer <= (1 << 64)-1 and str(integer) == value
    return value


def validate_owners(listing,container,generation,a,b=None):
    assert publication_generation(generation) == listing['owner_generation']
    assert container['state'] == 'running' and container['surface'] == {'kind':'xpra','port':14500,'exposure_id':a['id']}
    expected = [a] if b is None else [a,b]
    assert sorted(listing['exposures'],key=lambda row:row['id']) == sorted(expected,key=lambda row:row['id']), 'original exposures changed'


class Preview:
    """Original preview socket belongs to the canonical transport, no bearer."""
    def __init__(self,cycle,exposure,port,hits,*,deadline=None):
        self.cycle,self.exposure,self.port,self.hits = cycle,exposure,port,hits
        self.deadline = deadline if deadline is not None else time.monotonic()+cycle.cfg['timeout_seconds']
        self.transport = Transport(host='127.0.0.1',port=port)

    def request(self,method,path,*,cookie=None,body=None,form=False):
        headers = {'Host':f'{self.exposure}.localhost:{self.port}'}
        if cookie is not None:
            headers['Cookie'] = cookie
        if form:
            headers['Content-Type'] = 'application/x-www-form-urlencoded'
        remaining = self.deadline-time.monotonic()
        if remaining <= 0:
            raise TimeoutError('preview control deadline exceeded')
        result = self.transport.measure_request(method,path,headers=headers,body=body,
            timeout=remaining,max_response_bytes=self.cycle.cfg['max_response_bytes'])
        assert result.attempts == 1
        return result

    def retired(self):
        self.transport.expect_peer_close(timeout=self.deadline-time.monotonic())

    def close(self):
        self.transport.close()


def refused(preview,cookie,*,before,status):
    # Admission is connection scoped. A control with another/no cookie must
    # start a new socket, never inherit an already admitted browser flow.
    preview.close()
    result = preview.request('GET','/final',cookie=cookie)
    assert result.status == status, 'refusal was not the matched preview contract'
    assert preview.hits() == before, 'refused preview reached guest'


def require_preview_metrics(metrics,cfg,quick,labels=('service','gateway')):
    count = sample_count(cfg,'cycles',quick)
    for label in labels:
        for operation,entry in cfg['endpoints'].items():
            if entry['target_scope'] == 'gateway' and label != 'gateway':
                assert not any(key.startswith(label+'.'+operation+' [') for key in metrics), 'UDS mint must never count as coverage'
                continue
            for case in entry['cases']:
                for suffix in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                    key = f'{label}.{operation} [{case}].{suffix}'
                    if key not in metrics or len(metrics[key]['samples']) != count:
                        raise ValueError('missing or partial preview samples')
        for index in range(count):
            key = f'{label}.preview_controls.{index}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing or partial preview resources')


def collect_preview_controls(quick):
    from .route_preview_fixture import preview_target
    from .route_preview_flow import sample_preview
    cfg,metrics = preview_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with preview_target(label,cfg) as (target,service,reference,certificate,gateway):
                transport,gw_transport = target.transport_factory(),gateway.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                gw_cycle = Cycle(gw_transport,gateway,service,cfg,metrics)
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.preview_controls.{index}',Process(service.proc.pid),
                        partial(sample_preview,cycle,gw_cycle,reference,certificate))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
                    gw_transport.close()
    require_preview_metrics(metrics,cfg,quick)
    return metrics
