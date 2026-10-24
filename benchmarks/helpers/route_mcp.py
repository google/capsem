"""Finite live MCP discovery/calls, with existing transport and resource owners."""
import re
import time
import tomllib
from functools import partial
from pathlib import Path

from capsem_builder.crypto import uuid_v4
from psutil import Process

from .route_credentials import require_credential_metrics
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count

REFRESH = 'POST /mcp/servers/{server_id}/refresh'
CALL = 'POST /mcp/servers/{server_id}/tools/{tool_id}/call'
TOOLS = '/mcp/servers/local/tools/list'


def mcp_config():
    from capsem_builder.gate.benchmarkmcpschema import McpRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return McpRouteConfig.model_validate(raw['benchmark']['routes']['mcp_controls']).model_dump()


def validate_tools(value,cfg):
    assert len(value) == 4 and {row['original_name'] for row in value} == set(cfg['tools'])
    for row in value:
        name = row['original_name']
        assert set(row) == {'namespaced_name','original_name','description','server_name','annotations',
            'pin_hash','pin_changed','permission_action','permission_source'}
        expected = cfg['tools'][name]
        assert row['namespaced_name'] == 'local__'+name and row['server_name'] == 'local'
        assert row['description'] == expected['description']
        assert row['annotations'] == {'title':expected['title'],'readOnlyHint':True,
            'destructiveHint':False,'idempotentHint':True,'openWorldHint':name != 'echo'}
        assert row['permission_action'] == ('allow' if name in ('http_headers','fetch_http') else 'block')
        assert row['permission_source'] == 'settings'
        assert row['pin_changed'] is False and re.fullmatch(r'[0-9a-f]{64}',row['pin_hash'])


def expected_text(path,arguments,cfg):
    """Independent fixed mock corpus, never derived from the returned tool text."""
    url = arguments['url']
    if path.endswith('/local__http_headers/call'):
        return f"URL: {url}\nStatus: 200 OK\n\nHeaders:\n  content-type: {cfg['expected_content_type']}\n"
    return f"URL: {url}\nDomain: 127.0.0.1\nContent length: {len(cfg['expected_body'])}\n\n{cfg['expected_body']}"


def validate_result(value,path,arguments,cfg):
    assert value['jsonrpc'] == '2.0' and 'error' not in value and value['id'] is not None
    result = value['result']
    assert result.get('isError',False) is False and len(result['content']) == 1
    content = result['content'][0]
    assert content['type'] == 'text'
    text,expected = content['text'],expected_text(path,arguments,cfg)
    if path.endswith('/local__http_headers/call'):
        prefix,_,_ = expected.partition('Headers:\n')
        assert text.startswith(prefix+'Headers:\n')
        headers = text.split('Headers:\n',1)[1].splitlines()
        selected = [line.strip() for line in headers if line.strip().startswith('content-type:')]
        assert selected == ['content-type: '+cfg['expected_content_type']]
    else:
        assert text == expected, 'original raw MCP body differs'


def sample_mcp(cycle,vm_id,probe,base_url):
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    probe.ready(vm_id,deadline=deadline)
    assert cycle.read(TOOLS,deadline=deadline) == [], 'fresh owner already has cached discovery'
    def discovered(value):
        assert value == {'success':True,'server_id':'local','instances':1}, 'refresh did not reach exactly one live owner'
        probe.ready(vm_id,deadline=deadline)
        rows = cycle.read(TOOLS,deadline=deadline)
        validate_tools(rows,cfg)
        return rows
    original = cycle.observe_control(REFRESH,'discovered-live',
        lambda:cycle.call('POST','/mcp/servers/local/refresh',{},deadline=deadline),discovered,deadline=deadline)
    for tool,case in (('http_headers','headers'),('fetch_http','raw-body')):
        deadline = time.monotonic()+cfg['timeout_seconds']
        query = 'route_mcp='+uuid_v4().hex
        url = base_url+cfg['path']+'?'+query
        path = '/mcp/servers/local/tools/local__'+tool+'/call'
        arguments = {'url':url,'method':'GET'} if tool == 'http_headers' else {'url':url,'format':'raw','start_index':0,'max_length':len(cfg['expected_body'])+1}
        before = probe.before()
        def delivered(value,*,path=path,arguments=arguments,before=before,url=url,deadline=deadline):
            validate_result(value,path,arguments,cfg)
            probe.delivered(before,url,deadline=deadline)
            probe.ready(vm_id,deadline=deadline)
            assert cycle.read(TOOLS,deadline=deadline) == original, 'tool definition changed during call'
        cycle.observe_control(CALL,case,lambda path=path,arguments=arguments,deadline=deadline:
            cycle.call('POST',path,arguments,deadline=deadline),delivered,deadline=deadline)


def require_mcp_metrics(metrics,cfg,quick,labels=('service','gateway')):
    # Same finite observation validator, with this domain's resource key. No
    # statistics engine or copying sample arrays into a new representation.
    require_credential_metrics(metrics,cfg,quick,labels,resource_domain='mcp_controls')


def collect_mcp_controls(quick):
    from .route_mcp_fixture import mcp_target
    cfg,metrics = mcp_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with mcp_target(label,cfg) as (target,service,reference,certificate,probe_factory,base_url):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                probe = probe_factory(cycle)
                try:
                    deadline = time.monotonic()+cfg['timeout_seconds']
                    assert cycle.listing(deadline=deadline) == []
                    _,value = cycle.call('POST','/vms/create',{'name':'route-mcp','persistent':True,
                        'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],'container':{'image':reference,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
                    vm_id = cycle.identify(value)
                    cycle.ready(vm_id,reference,deadline=deadline)
                    assert service.proc is not None
                    measure(metrics,f'{label}.mcp_controls.{index}',Process(service.proc.pid),
                        partial(sample_mcp,cycle,vm_id,probe,base_url))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_mcp_metrics(metrics,cfg,quick)
    return metrics
