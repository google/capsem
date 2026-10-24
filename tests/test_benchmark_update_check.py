"""SOURCE doubles for finite check effects; never updater/native performance."""
import json
import time
from types import SimpleNamespace

import pytest
from capsem_builder.crypto import sha256
from helpers.http_measurement import MeasuredResponse
from helpers.update_release import _write_manifest_metadata

from benchmarks.helpers import route_update_check as subject


@pytest.mark.parametrize('fault',['none','failed','development','no-fetch','replayed','old-cache',
    'wrong-source','wrong-hash','invalid','wrong-binary','wrong-runtime','status','status-error','validation-error','retry','partial','timeout','output'])
def test_original_cli_check_requires_request_cache_and_independent_status(tmp_path,monkeypatch,fault):
    cfg = subject.update_check_config()
    url,body = 'http://127.0.0.1:1'+cfg['manifest_path'],b'{"independent":"manifest"}'
    _write_manifest_metadata(tmp_path,url)
    cache_path = tmp_path/'assets/manifest-metadata.json'
    original = json.loads(cache_path.read_bytes())
    requests,metrics = [],{}
    service = SimpleNamespace(home_dir=tmp_path,update_cli=tmp_path/'bin/capsem')
    cycle = subject.Cycle(None,SimpleNamespace(label='service'),service,cfg,metrics)
    current = {**original,'checked_url':url,'channel_hash':sha256(body).hexdigest(),
        'checked_at':int(time.time()),'validation_status':'valid','validation_error':None,
        'latest_version':cfg['binary_version'],'update_available':True,
        'latest_assets':cfg['runtime_revision'],'latest_images':cfg['runtime_revision'],
        'assets_update_available':True,'images_update_available':True}
    def call(method,path,request,*,deadline):
        assert (method,path,request) == ('POST','/update/check',{})
        assert deadline > time.monotonic()
        if fault == 'timeout':
            raise TimeoutError('original child check timed out')
        if fault != 'no-fetch':
            requests.extend([cfg['manifest_path']]*(2 if fault == 'replayed' else 1))
        for which,key,value in [('wrong-source','checked_url','other'),('wrong-hash','channel_hash','bad'),
            ('invalid','validation_status','invalid'),('wrong-binary','latest_version','0'),
            ('wrong-runtime','latest_assets','0')]:
            if fault == which:
                current[key] = value
        if fault != 'old-cache':
            cache_path.write_text(json.dumps(current))
        value = {'status':'failed' if fault == 'failed' else 'succeeded','exit_code':0,
            'command':{'program':str(service.update_cli),'args':['update','--check']},
            'stdout':'Development build detected' if fault == 'development' else 'Real update check',
            'stderr':''}
        if fault == 'output':
            value['stdout'] = 'x'*(cfg['max_cli_output_bytes']+1)
        return MeasuredResponse(200,{},b'{}',1,None if fault == 'partial' else 2,3,
            attempts=2 if fault == 'retry' else 1),value
    def read(path,*,deadline):
        assert path == '/update/status' and deadline > time.monotonic()
        status = {'channel_url':url,'channel_hash':sha256(body).hexdigest(),'checked_at':current['checked_at'],
            'validation_status':'valid','stale':False,
            'binary':{'latest':'wrong' if fault == 'status' else cfg['binary_version'],'update_available':True},
            'assets':{'latest':cfg['runtime_revision'],'update_available':True},
            'images':{'latest':cfg['runtime_revision'],'update_available':True}}
        if fault in {'status-error','validation-error'}:
            status['last_error' if fault == 'status-error' else 'validation_error'] = 'original check error'
        return status
    monkeypatch.setattr(cycle,'call',call)
    monkeypatch.setattr(cycle,'read',read)
    if fault == 'none':
        subject.sample_update_check(cycle,requests,body,url)
        assert len(metrics) == 4 and all(len(metric['samples']) == 1 for metric in metrics.values())
    else:
        with pytest.raises((AssertionError,ValueError,KeyError,TimeoutError)):
            subject.sample_update_check(cycle,requests,body,url)
        assert not metrics, 'failed original operation emitted successful samples'


def test_update_inputs_refuse_fallback_before_any_fixture(tmp_path,monkeypatch):
    monkeypatch.delenv('CAPSEM_RELEASE_BIN_DIR',raising=False)
    with pytest.raises(RuntimeError,match='prepared CLI'):
        subject.require_update_inputs()
    binary = tmp_path/'bin/capsem'
    binary.parent.mkdir()
    binary.write_bytes(b'explicit source-only CLI double, never executed')
    binary.chmod(0o755)
    monkeypatch.setenv('CAPSEM_RELEASE_BIN_DIR',str(binary.parent))
    monkeypatch.delenv('CAPSEM_ASSETS_DIR',raising=False)
    with pytest.raises(RuntimeError,match='prepared manifest'):
        subject.require_update_inputs()
    assets = tmp_path/'assets'
    assets.mkdir()
    monkeypatch.setenv('CAPSEM_ASSETS_DIR',str(assets))
    graph = {'packages':[{'status':'current','version':'0.7.1'}],'runtime':{'revision':'2026.0101.1'}}
    (assets/'manifest.json').write_text(json.dumps(graph))
    cli,root,read = subject.require_update_inputs()
    assert cli == binary and root == assets and read == graph
    (assets/'manifest.json').unlink()
    with pytest.raises(AssertionError,match='regular file'):
        subject.require_update_inputs()


def test_check_metric_inventory_requires_complete_receipts_and_resource_owner():
    cfg = subject.update_check_config()
    with pytest.raises(ValueError,match='missing or partial'):
        subject.require_credential_metrics({},cfg,True,resource_domain='update_check')


def test_missing_update_input_stops_main_before_other_collectors(monkeypatch):
    import runpy
    import sys
    from pathlib import Path
    namespace = runpy.run_path(str(Path(__file__).parents[1]/'benchmarks/collectors/routes'))
    globals_ = namespace['main'].__globals__
    calls = []
    monkeypatch.setattr(sys,'argv',['routes','--quick'])
    monkeypatch.delenv('CAPSEM_RELEASE_BIN_DIR',raising=False)
    for name in list(globals_):
        if name.startswith('collect_'):
            monkeypatch.setitem(globals_,name,lambda *_:calls.append('unexpected fixture'))
    with pytest.raises(RuntimeError,match='prepared CLI'):
        namespace['main']()
    assert calls == []


def test_update_case_inventory_cannot_be_relabelled_as_apply_or_concurrent():
    from capsem_builder.gate.benchmarkupdateschema import UpdateCheckConfig
    cfg = subject.update_check_config()
    cfg['endpoints'] = {'POST /update/apply':next(iter(cfg['endpoints'].values()))}
    with pytest.raises(ValueError,match='original real CLI'):
        UpdateCheckConfig.model_validate(cfg)


@pytest.mark.parametrize('fault',['start','cleanup','extra-fetch'])
def test_fixture_teardown_failure_or_late_request_never_certifies_success(tmp_path,monkeypatch,fault):
    from contextlib import contextmanager
    cfg = subject.update_check_config()
    home = tmp_path/'owned'
    home.mkdir()
    stopped,requests = [],[]
    class Service:
        home_dir = home
        def __init__(self,**kwargs):
            assert kwargs['update_cli'] == tmp_path/'selected-cli'
        def start(self):
            if fault == 'start':
                raise RuntimeError('original startup failed')
        def stop(self):
            stopped.append(True)
            if fault == 'cleanup':
                raise RuntimeError('original cleanup failed')
    @contextmanager
    def server(files):
        assert set(files) == {cfg['manifest_path']}
        yield 'http://127.0.0.1:1',requests
    def clone(source,destination):
        destination.mkdir()
        (destination/'manifest.json').write_text('{}')
        return destination
    monkeypatch.setattr(subject,'ServiceInstance',Service)
    monkeypatch.setattr(subject,'clone_benchmark_assets',clone)
    monkeypatch.setattr(subject,'_serve_manifest_release',server)
    monkeypatch.setattr(subject,'service_target',lambda service,label:'owned-target')
    graph = {'packages':[{'status':'current','version':'0.7.1'}],'runtime':{'revision':'2026.0101.1'}}
    with (
        pytest.raises((RuntimeError,AssertionError)),
        subject.update_target('service',cfg,(tmp_path/'selected-cli',tmp_path/'assets',graph)),
    ):
        requests.extend([cfg['manifest_path']]*(2 if fault == 'extra-fetch' else 1))
    assert stopped == [True]
