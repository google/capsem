"""Owned VM-free service/mock fixture; original process/listener effects only."""
import json
import socket
import time
from contextlib import contextmanager
from urllib.parse import urlsplit
from uuid import UUID

import tomli_w
from helpers.benchmark_assets import clone_benchmark_assets
from helpers.constants import ASSETS_DIR
from helpers.http_transport import Transport
from helpers.mock_server import start_mock_server, stop_process
from helpers.process_snapshot import process_tree_snapshot
from helpers.service import ServiceInstance
from psutil import Process

from .route_requests import Target, json_request, service_target


class Probe:
    clock = staticmethod(time.monotonic)

    def __init__(self, service, target, cfg, transcript):
        self.service, self.target, self.cfg, self.transcript = service, target, cfg, transcript
        self.transport = target.transport_factory()
        self.before = set()

    def workers(self):
        assert self.service.proc is not None
        rows = process_tree_snapshot(Process(self.service.proc.pid)).processes
        return {key for key,row in rows.items() if row.name == self.cfg['worker_binary']}

    def call(self, method, path, body, deadline):
        remaining = deadline-self.clock()
        if remaining <= 0:
            raise TimeoutError('proxy control deadline exceeded')
        if path == '/proxies':
            self.before = self.workers()
        return json_request(self.transport,self.target,method,path,body,
            timeout=remaining,maximum=self.cfg['max_response_bytes'])

    def capture(self, value):
        assert set(value) == {'session_id','provider','bind','port','base_url','lease_token','lease_expires_unix_ms'}, 'proxy receipt shape invalid'
        ident = value['session_id']
        assert ident.startswith('proxy-') and UUID(hex=ident[6:]).hex == ident[6:], 'proxy identity invalid'
        assert value['provider'] == self.cfg['provider'] and value['bind'] == '127.0.0.1'
        assert type(value['port']) is int and 0 < value['port'] <= 65535
        assert value['base_url'] == f"http://127.0.0.1:{value['port']}/v1", 'proxy listener identity differs'
        assert isinstance(value['lease_token'],str) and 0 < len(value['lease_token']) <= 128
        assert type(value['lease_expires_unix_ms']) is int and value['lease_expires_unix_ms'] > 0
        added = self.workers()-self.before
        assert len(added) == 1, 'create did not materialize one original owned proxy worker'
        directory = self.service.tmp_dir / 'sessions' / ident
        assert directory.is_dir() and not directory.is_symlink(), 'proxy session not materialized'
        return {'value':value,'identity':added.pop(),'directory':directory}

    def records(self):
        if not self.transcript.exists():
            return []
        with self.transcript.open('rb') as stream:
            body = stream.read(self.cfg['max_log_bytes']+1)
        assert len(body) <= self.cfg['max_log_bytes'], 'proxy mock transcript budget exceeded'
        return [json.loads(line) for line in body.splitlines() if line]

    def wait(self, until, deadline):
        if until >= deadline:
            raise TimeoutError('lease verification cannot fit deadline')
        while self.clock() < until:
            time.sleep(max(0,min(self.cfg['poll_seconds'],until-self.clock())))

    def model(self, owner, deadline):
        assert owner['identity'] in self.workers(), 'original proxy worker replaced or lost'
        before = len(self.records())
        url = urlsplit(owner['value']['base_url'])
        payload = {'model':self.cfg['model'],'messages':[{'role':'user','content':'benchmark lease proof'}],'stream':False}
        transport = Transport(host='127.0.0.1',port=url.port)
        try:
            target = Target('model',lambda:transport,{'Authorization':'Bearer sk-capsem-benchmark-synthetic'}, {})
            _, value = json_request(transport,target,'POST',url.path+'/chat/completions',payload,
                timeout=deadline-self.clock(),maximum=self.cfg['max_response_bytes'])
        finally:
            transport.close()
        assert value['choices'][0]['message']['content'] == self.cfg['expected_content'], 'proxy model bytes differ'
        rows = self.records()[before:]
        assert len(rows) == 1 and rows[0]['method'] == 'POST' and rows[0]['path'] == '/v1/chat/completions', 'proxy upstream delivery missing or duplicated'
        assert rows[0]['headers']['host'] == 'api.openai.com', 'proxy upstream identity differs'
        assert json.loads(rows[0]['request_body']) == payload, 'proxy upstream payload differs'
        assert owner['identity'] in self.workers(), 'proxy worker changed during model request'

    def settled(self, owner, deadline):
        while True:
            workers = self.workers()
            with socket.socket() as probe:
                probe.settimeout(min(self.cfg['poll_seconds'],max(.001,deadline-self.clock())))
                closed = probe.connect_ex(('127.0.0.1',owner['value']['port'])) != 0
            if closed and owner['identity'] not in workers and not owner['directory'].exists():
                return
            if self.clock() >= deadline:
                raise TimeoutError('original proxy listener/process/session did not settle')
            time.sleep(max(0,min(self.cfg['poll_seconds'],deadline-self.clock())))


@contextmanager
def proxy_target(label,cfg):
    service, mock, probe = ServiceInstance(isolated_corp_config=True), None, None
    try:
        transcript = service.tmp_dir / 'proxy-upstream.jsonl'
        mock, ready = start_mock_server(request_log=transcript)
        service.assets_dir = clone_benchmark_assets(ASSETS_DIR,service.home_dir/'benchmark-assets')
        (service.home_dir/'settings.toml').write_text(tomli_w.dumps({'images':{'catalog':False},'settings':{'app.auto_update':{'value':False,'modified':'route-fixture'}}}))
        (service.home_dir/'corp.toml').write_text(tomli_w.dumps({
            'refresh_policy':'24h', 'network':{'upstream_overrides':{'api.openai.com:443':{'dial':ready['http_addr'],'protocol':'http'}}},
            'corp':{'rules':{'allow_standalone_fixture':{'name':'allow_standalone_fixture','action':'allow','priority':-100,
                'detection_level':'informational','reason':'Allow the hermetic standalone model fixture.',
                'match':'http.host == "api.openai.com" && tcp.port == "443"'}}}}))
        service.start()
        probe = Probe(service,service_target(service,label),cfg,transcript)
        yield probe
    finally:
        if probe is not None:
            probe.transport.close()
        try:
            service.stop()  # owns uncertain admission cleanup; no mutation replay
        finally:
            stop_process(mock)
