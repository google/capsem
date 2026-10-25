"""Existing lifecycle readiness/resource seams; no native fixture execution."""
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_lifecycle
from benchmarks.helpers import route_save as subject


def test_readiness_defaults_to_persistent_but_explicit_ephemeral_is_supported(monkeypatch):
    cfg = subject.save_config()
    cycle = subject.Cycle(None,None,None,cfg,{})
    cycle.owned.add('original')
    row = {'id':'original','status':'Running','persistent':False,'pid':123}
    monkeypatch.setattr(cycle,'info',lambda *args,**kwargs:row)
    monkeypatch.setattr(cycle,'read',lambda *args,**kwargs:{'state':'running','image':'fixture@sha256:pinned','digest':'sha256:pinned'})
    monkeypatch.setattr(cycle,'execute',lambda *args,**kwargs:(None,{'exit_code':0,'truncated':False,
        'stdout':{'encoding':'utf8','data':'CAPSEM-UID-MAP 0 100000 65536\nready\n'},'stderr':{'encoding':'utf8','data':''}}))
    with pytest.raises(AssertionError):
        cycle.ready('original','fixture@sha256:pinned',deadline=time.monotonic()+1)
    assert cycle.ready('original','fixture@sha256:pinned',deadline=time.monotonic()+1,persistent=False) == row
    with pytest.raises(AssertionError):
        route_lifecycle.validate_running({**row,'persistent':True},'original',persistent=False)


def test_policy_grants_only_known_export_and_keeps_other_settings(tmp_path):
    import tomllib
    (tmp_path/'settings.toml').write_text('[appearance]\ntheme="dark"\n')
    subject.prepare_policy(SimpleNamespace(home_dir=tmp_path))
    value = tomllib.loads((tmp_path/'settings.toml').read_text())
    assert value['appearance'] == {'theme':'dark'}
    assert set(value['profiles']['rules']) == {'route_save_export'}
    rule = value['profiles']['rules']['route_save_export']
    assert rule['action'] == 'allow' and rule['match'] == 'file.export.name == "capsem-route-save-proof"'


@pytest.mark.parametrize('broken',[False,True])
def test_owned_fixture_sampler_and_original_failure_teardown(monkeypatch,broken):
    from benchmarks.helpers import route_lifecycle_fixture
    events = []
    @contextmanager
    def fixture(label,*,prepare):
        assert prepare is subject.prepare_policy
        try:
            yield SimpleNamespace(transport_factory=lambda:SimpleNamespace(close=lambda:events.append('close'))),SimpleNamespace(proc=SimpleNamespace(pid=123)),'reference',None
        finally:
            events.append('stop-owner')
    class Cycle:
        def __init__(self,*args):
            pass
        def cleanup(self):
            events.append('drain')
        def listing(self):
            return []
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',fixture)
    monkeypatch.setattr(subject,'Cycle',Cycle)
    monkeypatch.setattr(subject,'Process',lambda pid:('original',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original',123)
        if broken:
            raise ValueError('partial save')
        events.append(phase)
    monkeypatch.setattr(subject,'measure',measure)
    monkeypatch.setattr(subject,'require_save_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='partial'):
            subject.collect_save_controls(True)
        assert events == ['close','stop-owner']
    else:
        subject.collect_save_controls(True)
        assert events == ['service.save_controls.0','drain','close','stop-owner',
            'gateway.save_controls.0','drain','close','stop-owner','counts']
