"""Live-owner effects modeled explicitly; no MCP/service process launches."""
import json
import re
from pathlib import Path
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_mcp as subject


class Backend:
    def __init__(self,failure=''):
        self.failure,self.tools,self.calls = failure,[],[]
        self.cfg = subject.mcp_config()
        if failure == 'cached-before':
            self.tools = tools(self.cfg)

    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        self.calls.append((method,path))
        if method == 'GET':
            value = self.tools
        elif path.endswith('/refresh'):
            value = {'success':True,'server_id':'local','instances':0 if self.failure == 'no-owner' else 1}
            if self.failure != 'refresh-noop':
                self.tools = tools(self.cfg)
            if self.failure == 'wrong-tool':
                self.tools[0]['description'] = 'forged'
        else:
            value = {'jsonrpc':'2.0','id':1,'result':{'content':[{'type':'text','text':subject.expected_text(path,body,self.cfg)}]}}
            if self.failure == 'tool-error':
                value['error'] = {'code':-1,'message':'refused'}
            if self.failure == 'wrong-body':
                value['result']['content'][0]['text'] = 'forged'
        return MeasuredResponse(200,{},b'{}',1,2,3,attempts=2 if self.failure == 'retry' else 1),value


def tools(cfg):
    return [{'namespaced_name':'local__'+name,'original_name':name,'server_name':'local',
        'description':metadata['description'],'annotations':{'title':metadata['title'],'readOnlyHint':True,
            'destructiveHint':False,'idempotentHint':True,'openWorldHint':name != 'echo'},
        'pin_hash':'a'*64,'pin_changed':False,'permission_action':'allow' if name in ('http_headers','fetch_http') else 'block',
        'permission_source':'settings'} for name,metadata in cfg['tools'].items()]


class Probe:
    def __init__(self,failure=''):
        self.failure,self.observed = failure,[]
    def ready(self,vm,*,deadline):
        assert self.failure != 'dead-worker', 'original MCP workers retired'
    def before(self):
        return len(self.observed)
    def delivered(self,before,url,*,deadline):
        assert self.failure != 'no-delivery', 'no actual upstream request'
        self.observed.append(url)
        assert len(self.observed) == before+1


def setup(monkeypatch,failure=''):
    backend,probe = Backend(failure),Probe(failure)
    cycle = subject.Cycle(None,SimpleNamespace(label='service'),None,backend.cfg,{})
    monkeypatch.setattr(cycle,'call',backend.call)
    return cycle,backend,probe


def test_original_live_refresh_and_narrow_calls_have_distinct_effects(monkeypatch):
    cycle,backend,probe = setup(monkeypatch)
    subject.sample_mcp(cycle,'owned',probe,'http://127.0.0.1:3713')
    assert len(cycle.metrics) == 12 and len(probe.observed) == 2
    assert sum(path.endswith('/refresh') for _,path in backend.calls) == 1
    assert sum(path.endswith('/call') for _,path in backend.calls) == 2


@pytest.mark.parametrize('failure',['no-owner','refresh-noop','wrong-tool','tool-error','retry','dead-worker','no-delivery','cached-before','wrong-body'])
def test_empty_cached_refused_or_dead_owner_cannot_pass(monkeypatch,failure):
    cycle,_,probe = setup(monkeypatch,failure)
    with pytest.raises((AssertionError,ValueError)):
        subject.sample_mcp(cycle,'owned',probe,'http://127.0.0.1:3713')


def test_corpus_is_exact_shipped_builtin_description_and_mock_body():
    root = Path(__file__).parents[1]
    source = (root/'crates/capsem-mcp-builtin/src/main.rs').read_text()
    definitions = dict(re.findall(r'name = "([^"]+)",\n        description = "([^"]+)"',source))
    cfg = subject.mcp_config()
    assert definitions == {name:metadata['description'] for name,metadata in cfg['tools'].items()}
    assert 'b'+json.dumps(cfg['expected_body']) in (root/'crates/capsem-mock-server/src/main.rs').read_text()


def test_bounds_metadata_and_partial_samples_are_not_optional(monkeypatch):
    from capsem_builder.gate.benchmarkmcpschema import McpRouteConfig
    cfg = subject.mcp_config()
    for changed in ({'cycles':9},{'quick_cycles':3},{'timeout_seconds':1},{'tools':{}},{'endpoints':{}},{'path':'/other'}):
        with pytest.raises(ValueError):
            McpRouteConfig.model_validate({**cfg,**changed})
    with pytest.raises(ValueError,match='partial'):
        subject.require_mcp_metrics({},cfg,True)
    cycle,_,probe = setup(monkeypatch)
    subject.sample_mcp(cycle,'owned',probe,'http://127.0.0.1:3713')
    cycle.metrics['service.mcp_controls.0.wall_s'] = {'samples':[1]}
    subject.require_mcp_metrics(cycle.metrics,cfg,True,labels=('service',))
    cycle.metrics[next(iter(cycle.metrics))]['samples'] *= 2
    with pytest.raises(ValueError,match='partial'):
        subject.require_mcp_metrics(cycle.metrics,cfg,True,labels=('service',))
