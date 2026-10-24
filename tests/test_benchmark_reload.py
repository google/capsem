"""Independent runtime-effect doubles; no service or native execution."""
import time
from types import SimpleNamespace
from typing import Any

import pytest

from benchmarks.helpers import route_reload as subject


@pytest.mark.parametrize('fault',['none','noop','bad-ack','wrong-count','boolean-count','parser','replacement','generation','not-restored','no-block','wrong-bytes'])
def test_original_policy_block_and_exact_restore(tmp_path,monkeypatch,fault):
    cfg = subject.reload_config()
    home,session = tmp_path/'home',tmp_path/'session'
    home.mkdir()
    (session/'vm').mkdir(parents=True)
    corp = home/'corp.toml'
    corp.write_text('[corp.rules.baseline]\naction="allow"\n')
    corp.chmod(0o600)
    (home/'settings.toml').write_text('[appearance]\ntheme="dark"\n')
    active = session/'vm/active_policy.toml'
    baseline = b'[corp_rules.corp.rules.baseline]\naction="allow"\n'
    active.write_bytes(baseline)
    state: dict[str,Any] = {'blocked':False,'calls':0,'traffic':[]}
    def call(method,path,body=None,**kwargs):
        if path == '/vms/create':
            return None,{'id':'owned'}
        assert path == '/corp/reload'
        state['calls'] += 1
        if fault != 'noop':
            state['blocked'] = state['calls'] == 1
            active.write_bytes(f'[corp_rules.corp.rules.{subject.RULE}]\naction="block"\n'.encode() if state['blocked'] else baseline)
        if fault == 'parser':
            active.write_bytes(b'bad = [')
        if fault == 'not-restored' and state['calls'] == 2:
            active.write_bytes(baseline+b'\n# divergent policy\n')
        return None,{'success':fault != 'bad-ack','reloaded':0 if fault == 'wrong-count' else True if fault == 'boolean-count' else 1}
    def observe(operation,case,request,verify,**kwargs):
        assert operation == subject.OPERATION
        _,value = request()
        return verify(value)
    cycle = SimpleNamespace(cfg=cfg,service=SimpleNamespace(home_dir=home),owned=set(),
        listing=lambda **kwargs:[],call=call,identify=lambda value:value['id'],
        ready=lambda *args,**kwargs:None,session=lambda *args,**kwargs:session,
        observe_control=observe,cleanup=lambda:state.update(cleaned=True))
    class Probe:
        def ready(self,vm_id,*,deadline):
            assert fault != 'replacement' or state['calls'] == 0, 'original owner replaced'
        def __call__(self,vm_id,*,allowed,deadline):
            assert deadline > time.monotonic()
            assert allowed is (not state['blocked'])
            assert fault != 'no-block' or allowed, 'blocked flow reached upstream'
            assert fault != 'wrong-bytes', 'original request bytes differ'
            state['traffic'].append(allowed)
    monkeypatch.setattr(subject,'generation',lambda *args:'1' if fault == 'generation' and state['calls'] else '18446744073709551615')
    original = {name:(home/name).read_bytes() for name in ('settings.toml','corp.toml')}
    if fault == 'none':
        subject.sample_reload(cycle,'reference',SimpleNamespace(read_text=lambda:'certificate'),Probe())
        assert state['calls'] == 2 and state['traffic'] == [True,False,True] and state['cleaned']
        assert active.read_bytes() == baseline
    else:
        with pytest.raises((AssertionError,ValueError)):
            subject.sample_reload(cycle,'reference',SimpleNamespace(read_text=lambda:'certificate'),Probe())
    assert {name:(home/name).read_bytes() for name in original} == original
    assert corp.stat().st_mode & 0o777 == 0o600


def test_config_exact_finite_and_no_missing_metrics():
    cfg = subject.reload_config()
    assert cfg['endpoints'][subject.OPERATION]['cases'] == {'block-original':'cycles','restore-original':'cycles'}
    with pytest.raises(ValueError):
        subject.require_reload_metrics({},cfg,True)
