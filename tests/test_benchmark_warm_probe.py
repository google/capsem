"""Synthetic protocol responses exercise the probe, never execute its guest code."""
import json
import shlex
import time
from contextlib import nullcontext
from types import SimpleNamespace

import msgpack
import pytest

from benchmarks.helpers import route_warm_probe as subject
from benchmarks.helpers.route_pause_resume import pause_resume_config


def result(value,*,mapping='0 100000 65536',truncated=False):
    return {'exit_code':0,'truncated':truncated,'stderr':{'encoding':'utf8','data':''},
        'stdout':{'encoding':'utf8','data':'CAPSEM-UID-MAP '+mapping+'\n'+json.dumps(value,sort_keys=True)+'\n'}}


@pytest.mark.parametrize('fault',['none','vm','truncated','wrong-nonce','wrong-memory','wrong-pid','oversize'])
def test_original_metadata_is_bounded_namespace_proved_and_exact(monkeypatch,fault):
    cfg = pause_resume_config()
    nonce = 'a'*32
    metadata = {'pid':0 if fault == 'wrong-pid' else 123,'memory':'bad' if fault == 'wrong-memory' else 'b'*32,
        'nonce':'foreign' if fault == 'wrong-nonce' else nonce}
    if fault == 'oversize':
        metadata['memory'] = 'b'*(cfg['max_metadata_bytes']+1)
    commands = []
    def execute(vm,command,*,deadline):
        assert deadline > time.monotonic()
        commands.append(shlex.split(command)[2])
        compile(commands[-1],'<warm-metadata-guest>','exec')
        return None,result(metadata,mapping='0 0 4294967295' if fault == 'vm' else '0 100000 65536',truncated=fault == 'truncated')
    monkeypatch.setattr(subject,'service_gateway',lambda service:'existing-owned-gateway')
    probe = subject.WarmProbe(SimpleNamespace(cfg=cfg,service='isolated',execute=execute),'original')
    probe.nonce = nonce
    if fault == 'none':
        assert probe.metadata(time.monotonic()+1,alive=True) == metadata
        assert 'os.kill' in commands[0] and str(cfg['max_metadata_bytes']) in commands[0]
    else:
        with pytest.raises(AssertionError):
            probe.metadata(time.monotonic()+1,alive=True)


@pytest.mark.parametrize('fault',['none','cold-screen','different-pid','missing-response','not-started'])
def test_original_foreground_must_answer_with_retained_ram(monkeypatch,fault):
    cfg = pause_resume_config()
    state = {'pid':123,'memory':'b'*32,'nonce':'a'*32}
    sent = []
    frames = [msgpack.packb({'stdout':b'warm-restored-'+(b'c' if fault == 'cold-screen' else b'b')*32}),
        msgpack.packb({'stdout':b'warm-finished-'+b'b'*32})]
    if fault == 'missing-response':
        frames = []
    class Socket:
        def send(self,value):
            sent.append(msgpack.unpackb(value)['stdin'])
        def recv(self,timeout):
            assert timeout > 0
            if not frames:
                raise TimeoutError('original foreground did not respond')
            return frames.pop(0)
    monkeypatch.setattr(subject,'service_gateway',lambda service:'existing-owned-gateway')
    probe = subject.WarmProbe(SimpleNamespace(cfg=cfg,service='isolated'),'original')
    probe.state = None if fault == 'not-started' else state
    monkeypatch.setattr(probe,'connection',lambda deadline:nullcontext(Socket()))
    monkeypatch.setattr(probe,'metadata',lambda *args,**kwargs:{**state,'pid':999} if fault == 'different-pid' else state)
    if fault == 'none':
        probe.resumed(time.monotonic()+1)
        assert sent == [b'c',b'q']
    else:
        with pytest.raises((AssertionError,TimeoutError)):
            probe.resumed(time.monotonic()+1)
        assert b'q' not in sent


def test_foreground_stimulus_compiles_and_checks_original_descriptions():
    source = subject.foreground('a'*32)
    compile(source,'<warm-original-foreground>','exec')
    for operation in ('os.urandom','os.getcwd','identity(held)==before_file',
        'identity(directory)==before_directory','dir_fd=directory','os.lseek(image',
        "Path('/etc/os-release').read_bytes()","os.read(0,1)==b'c'","os.read(0,1)==b'q'"):
        assert operation in source
    with pytest.raises(AssertionError):
        subject.foreground('not-a-nonce')
