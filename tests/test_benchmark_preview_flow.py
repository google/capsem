"""Explicit live-effect models exercise control wiring, never native proof."""
import time
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_preview as controls
from benchmarks.helpers import route_preview_flow as subject

VM = '123e4567-e89b-42d3-a456-426614174000'
GEN = '18446744073709551615'


def receipt(status=200,body=b'{}',headers=None):
    return MeasuredResponse(status,headers or {},body,1,2 if body else None,3,attempts=1)


class Backend:
    def __init__(self,fault):
        self.fault,self.created,self.b,self.count = fault,False,None,0
        self.a = {'id':'surface-a','host_port':None,'guest_port':14500,'target':'container','access':'http_preview'}
        self.sessions,self.tokens,self.calls = {},{},[]

    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        assert deadline is not None and deadline > time.monotonic()
        self.calls.append((method,path))
        if path == '/vms/list':
            result = {'sandboxes':[{'id':VM}] if self.created else []}
        elif path == '/vms/create':
            self.created,result = True,{'id':VM}
        elif path.endswith('/container'):
            result = {'state':'running','surface':{'kind':'xpra','port':14500,'exposure_id':'surface-a'}}
            if self.fault == 'changed-a' and self.b is not None:
                result['surface']['exposure_id'] = 'manual-b'
        elif path.endswith('/exposures'):
            if method == 'POST':
                self.b = {**self.a,'id':'manual-b','guest_port':8080}
                result = self.b
            else:
                result = {'owner_generation':GEN,'exposures':[self.a]+([self.b] if self.b else [])}
                if self.fault == 'create-noop':
                    result['exposures'] = [self.a]
        elif path.endswith(('/preview-session','/surface/session')):
            exposure = 'surface-a' if path.endswith('/surface/session') else path.split('/')[-2]
            if method == 'DELETE':
                self.sessions = {cookie:owner for cookie,owner in self.sessions.items() if owner != exposure}
                result = {'revoked':0 if self.fault == 'wrong-revoked' else 1}
            elif exposure == 'manual-b' and self.b is None:
                assert expected_status == 404
                return receipt(404),{'error':'exposure not found'}
            else:
                token = f'synthetic-{len(self.tokens)}'
                self.tokens[token] = [exposure,False]
                result = {'url':f'http://{exposure}.localhost:1234/_capsem/bootstrap',
                    'bootstrap_token':token,'expires_in_seconds':30}
                if self.fault == 'foreign-origin':
                    result['url'] = result['url'].replace(':1234',':9999')
        elif method == 'DELETE' and path.endswith('/manual-b'):
            if self.fault != 'delete-noop':
                self.b = None
            self.sessions = {cookie:owner for cookie,owner in self.sessions.items() if owner != 'manual-b'}
            result = {'success':True}
        else:
            raise AssertionError((method,path))
        measured = receipt()
        if self.fault == 'retry':
            measured = MeasuredResponse(200,{},b'{}',1,2,3,attempts=2)
        return measured,result


class Preview:
    def __init__(self,backend,cycle,exposure,port,hits,*,deadline):
        self.backend,self.cycle,self.exposure,self.port,self.hits = backend,cycle,exposure,port,hits
        self.admitted,self.closed = None,False
    def request(self,method,path,*,cookie=None,body=None,form=False):
        backend = self.backend
        if method == 'POST':
            from urllib.parse import parse_qs
            assert body is not None
            token = parse_qs(body.decode())['bootstrap_token'][0]
            exposure,used = backend.tokens[token]
            if used and backend.fault != 'replay':
                return receipt(400,b'refused')
            assert exposure == self.exposure
            backend.tokens[token][1] = True
            cookie = 'capsem_preview='+token
            backend.sessions[cookie] = exposure
            return receipt(303,b'',{'location':'/_capsem/surface/' if exposure == 'surface-a' else '/',
                'cache-control':'no-store','set-cookie':cookie+'; Path=/; HttpOnly; SameSite=Lax; Max-Age=900'})
        if self.admitted is None:
            if cookie is None:
                return receipt(401,b'refused')
            if backend.sessions.get(cookie) != self.exposure:
                return receipt(403,b'refused')
            self.admitted = cookie
        if path == '/_capsem/surface/':
            return receipt(200,b'<script src="capsem-surface.js"></script><script src="xpra-main.js"></script>',
                {'content-type':'text/html; charset=utf-8','content-security-policy':f"connect-src 'self' ws://{self.exposure}.localhost:{self.port}"})
        if path.startswith('/hello'):
            return receipt(200,b'wrong' if backend.fault == 'port-bytes' else b'workload:GET /hello?from=kingslanding HTTP/1.1')
        backend.count += 2 if backend.fault == 'duplicate-hit' else 1
        return receipt(200,b'redirected')
    def retired(self):
        assert self.admitted is not None and self.admitted not in self.backend.sessions
        assert self.backend.fault != 'flow-leak', 'original socket remains live'
        self.close()
    def close(self):
        self.admitted,self.closed = None,True


def setup(monkeypatch,tmp_path,label,fault=''):
    backend,cfg = Backend(fault),controls.preview_config()
    (tmp_path/'preview.port').write_text('1234')
    cert = tmp_path/'cert'
    cert.write_text('synthetic')
    cycle = controls.Cycle(None,SimpleNamespace(label=label),SimpleNamespace(tmp_dir=tmp_path),cfg,{})
    gateway = controls.Cycle(None,SimpleNamespace(label='gateway'),None,cfg,cycle.metrics)
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(gateway,'call',backend.call)
    monkeypatch.setattr(cycle,'ready',lambda *args,**kw:None)
    monkeypatch.setattr(subject,'start_workload',lambda *args:lambda:backend.count)
    peers = []
    def peer(*args,**kw):
        value = Preview(backend,*args,**kw)
        peers.append(value)
        return value
    monkeypatch.setattr(subject,'Preview',peer)
    return cycle,gateway,backend,cert,peers


@pytest.mark.parametrize('label',['service','gateway'])
def test_five_original_operations_preserve_surface_and_distinct_target_metrics(monkeypatch,tmp_path,label):
    cycle,gateway,backend,cert,peers = setup(monkeypatch,tmp_path,label)
    subject.sample_preview(cycle,gateway,'pinned',cert)
    assert len(cycle.metrics) == (12 if label == 'service' else 20)
    cycle.metrics[label+'.preview_controls.0.wall_s'] = {'samples':[1]}
    controls.require_preview_metrics(cycle.metrics,cycle.cfg,True,labels=(label,))
    assert backend.b is None and all(peer.closed for peer in peers)
    assert backend.calls.count(('DELETE',f'/vms/{VM}/exposures/manual-b/preview-session')) == 1
    assert backend.calls.count(('DELETE',f'/vms/{VM}/exposures/manual-b')) == 1
    assert sum(path.endswith('/surface/session') for _,path in backend.calls) == 1


@pytest.mark.parametrize('fault',['create-noop','wrong-revoked','changed-a','delete-noop','foreign-origin',
    'replay','port-bytes','duplicate-hit','flow-leak','retry'])
def test_echo_noop_leak_replay_or_wrong_original_effect_cannot_pass(monkeypatch,tmp_path,fault):
    cycle,gateway,_,cert,peers = setup(monkeypatch,tmp_path,'gateway',fault)
    with pytest.raises((AssertionError,ValueError)):
        subject.sample_preview(cycle,gateway,'pinned',cert)
    assert all(peer.closed for peer in peers)
