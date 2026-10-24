"""Finite source doubles for VM-only execution and actual stopped removal."""
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_run_purge as subject


def receipt():
    return MeasuredResponse(200,{},b'{}',1,2,3)


def test_config_pins_two_original_operations_and_finite_cases():
    from capsem_builder.gate.benchmarkrunpurgeschema import RunPurgeConfig
    cfg = subject.run_purge_config()
    assert set(cfg['endpoints']) == {'POST /run','POST /purge'}
    for changed in ({'cycles':5},{'quick_cycles':3},{'exec_timeout_seconds':900},{'marker':''},{'endpoints':{}}):
        with pytest.raises(ValueError):
            RunPurgeConfig.model_validate({**cfg,**changed})
    with pytest.raises(ValueError,match='partial'):
        subject.require_run_purge_metrics({},cfg,True)


class PurgeBackend:
    def __init__(self,tmp_path,fault=''):
        self.fault,self.rows,self.paths,self.processes,self.calls = fault,{},[],[],[]
        self.tmp_path,self.index = tmp_path,0
    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        assert deadline is not None and deadline > time.monotonic()
        self.calls.append((method,path,body))
        if path == '/vms/list':
            result = {'sandboxes':list(self.rows.values())}
        elif path == '/vms/create':
            assert body is not None
            self.index += 1
            vm = f'123e4567-e89b-42d3-a456-{self.index:012d}'
            self.rows[vm] = {'id':vm,'name':body['name'],'persistent':True,'pid':100+self.index,'status':'Running','can_resume':True}
            directory = self.tmp_path/vm
            directory.mkdir()
            self.paths.append(directory)
            result = {'id':vm}
        elif path.endswith('/info'):
            result = self.rows[path.split('/')[2]]
        elif path.endswith('/stop'):
            self.rows[path.split('/')[2]]['status'] = 'Stopped'
            self.processes[int(path.split('/')[2][-1])-1].alive = False
            result = {'success':True,'persistent':True}
        elif path.endswith('/files/list?depth=1'):
            result = {'entries':[{'path':subject.FILE,'name':subject.FILE,'type':'file','size':len(subject.expected_marker('purge'))}]}
        elif path == '/purge':
            assert body is not None
            if not body['all']:
                if self.fault == 'default-deletes':
                    self.rows.clear()
                result = {'purged':0,'persistent_purged':0,'ephemeral_purged':0}
            else:
                if self.fault != 'no-remove':
                    self.rows.clear()
                if self.fault != 'path-leak':
                    for directory in self.paths:
                        directory.rmdir()
                result = {'purged':1 if self.fault == 'count' else 2,'persistent_purged':2,'ephemeral_purged':0}
        else:
            raise AssertionError((method,path))
        return receipt(),result


@pytest.mark.parametrize('fault',['','default-deletes','no-remove','path-leak','count','wrong-bytes','vm-fallback'])
def test_purge_requires_known_workload_effect_stopped_retention_and_exact_disposal(monkeypatch,tmp_path,fault):
    backend = PurgeBackend(tmp_path,fault)
    cfg,metrics = subject.run_purge_config(),{}
    cycle = subject.Cycle(None,SimpleNamespace(label='service'),None,cfg,metrics)
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(cycle,'ready',lambda vm,*args,**kw:backend.rows[vm])
    monkeypatch.setattr(subject,'read_file',lambda *args:None)
    def process(pid):
        value = SimpleNamespace(alive=True)
        value.is_running = lambda:value.alive
        backend.processes.append(value)
        return value
    monkeypatch.setattr(subject,'Process',process)
    monkeypatch.setattr(cycle,'session',lambda vm,**kw:tmp_path/vm)
    def execute(vm,command,*,deadline):
        output = 'wrong' if fault == 'wrong-bytes' else subject.expected_marker('purge').decode().rstrip('\n')
        mapping = '0 0 4294967295' if fault == 'vm-fallback' else '0 100000 65536'
        return receipt(),{'exit_code':0,'truncated':False,
            'stdout':{'encoding':'utf8','data':f'CAPSEM-UID-MAP {mapping}\n{output}\n'},
            'stderr':{'encoding':'utf8','data':''}}
    monkeypatch.setattr(cycle,'execute',execute)
    certificate = tmp_path/'cert'
    certificate.write_text('synthetic')
    if fault:
        with pytest.raises((AssertionError,KeyError,ValueError)):
            subject.sample_purge(cycle,'pinned',certificate)
    else:
        subject.sample_purge(cycle,'pinned',certificate)
        assert cycle.owned == set() and not backend.rows and len(metrics) == 4
        assert [body for _,path,body in backend.calls if path == '/purge'] == [{'all':False},{'all':True}]


def test_policy_admits_only_original_synthetic_files(tmp_path):
    import tomllib
    subject.prepare_policy(SimpleNamespace(home_dir=tmp_path))
    rules = tomllib.loads((tmp_path/'settings.toml').read_text())['profiles']['rules']
    assert set(rules) == {'route_run_purge_export','route_run_purge_import'}
    assert rules['route_run_purge_export']['match'] == 'file.export.name == "capsem-route-purge-proof" || file.export.name == "capsem-route-run-proof"'
    assert rules['route_run_purge_import']['match'] == 'file.import.name == "capsem-route-run-release"'
    assert all(rule['action'] == 'allow' for rule in rules.values())


def test_file_condition_quotes_names_without_turning_them_into_policy():
    from helpers.settings_policy import file_name_condition
    assert file_name_condition('import', ['a" || true', 'b\\c']) == (
        'file.import.name == "a\\" || true" || file.import.name == "b\\\\c"')
    for action,names in [('import', []), ('unrecognized', ['file'])]:
        with pytest.raises(ValueError):
            file_name_condition(action,names)


@pytest.mark.parametrize('broken',[False,True])
def test_existing_fixture_sampler_finite_cycles_and_failure_cleanup(monkeypatch,broken):
    from benchmarks.helpers import route_lifecycle_fixture
    events = []
    @contextmanager
    def target(label,*,prepare):
        assert prepare is subject.prepare_policy
        try:
            yield SimpleNamespace(transport_factory=lambda:SimpleNamespace(close=lambda:events.append('close'))),SimpleNamespace(proc=SimpleNamespace(pid=123)),'reference',None
        finally:
            events.append('owner-stopped')
    class Cycle:
        def __init__(self,*args):
            pass
        def cleanup(self):
            events.append('drain')
        def listing(self):
            return []
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',target)
    monkeypatch.setattr(subject,'Cycle',Cycle)
    monkeypatch.setattr(subject,'Process',lambda pid:('original',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original',123)
        if broken:
            raise ValueError('partial run/purge')
        events.append(phase)
    monkeypatch.setattr(subject,'measure',measure)
    monkeypatch.setattr(subject,'require_run_purge_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='partial'):
            subject.collect_run_purge(True)
        assert events == ['close','owner-stopped']
    else:
        subject.collect_run_purge(True)
        assert events == ['service.run_purge.0','drain','close','owner-stopped',
            'gateway.run_purge.0','drain','close','owner-stopped','counts']
