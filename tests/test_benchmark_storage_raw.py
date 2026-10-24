"""Source-only original I/O capture/transport contracts; no VM or collector run."""
import copy
import io
import json
import runpy
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'guest/artifacts'))
from capsem_bench.helpers import BLOCK_1M, BLOCK_4K, RAND_IO_COUNT
from capsem_bench.io_capture import (
    Budget,
    Operations,
    capture,
    environment_budget,
    write_artifact,
)

from benchmarks.helpers.storage_raw import extract

LIMITS=SimpleNamespace(max_samples_per_metric=1000000,max_samples_per_run=4000000,max_collector_bytes=67108864)


def fact(count,size):
    return {'raw_io':{'count':count,'completed_bytes':size,'elapsed_seconds':0.123456789,
        'latencies_ms':[0.000123456789]*count}}


def disk_document(monkeypatch):
    monkeypatch.setenv('CAPSEM_BENCH_SIZE_MB','1')
    return {name:fact(RAND_IO_COUNT if name.startswith('rand_') else 1,
        RAND_IO_COUNT*BLOCK_4K if name.startswith('rand_') else BLOCK_1M)
        for name in ('seq_write','seq_read','rand_write_4k','rand_read_4k')}


def test_original_arrays_move_once_and_metadata_keeps_exact_facts(monkeypatch):
    document=disk_document(monkeypatch)
    original=document['rand_read_4k']['raw_io']['latencies_ms']
    metrics,facts=extract('disk',document,LIMITS)
    assert metrics['rand_read_4k.latency_ms']['samples'] is original
    assert metrics['rand_read_4k.latency_ms']['samples']==[0.000123456789]*RAND_IO_COUNT
    assert facts['rand_read_4k']=={'count':RAND_IO_COUNT,'completed_bytes':RAND_IO_COUNT*BLOCK_4K,'elapsed_seconds':0.123456789}
    assert 'raw_io' not in json.dumps(document)
    assert 'latencies_ms' not in json.dumps(facts)


@pytest.mark.parametrize('mutation',[
    'missing','count','length','bytes','nan','negative','bool','time','extra_field','extra_operation','error',
])
def test_invalid_original_capture_cannot_be_flattened_into_success(monkeypatch,mutation):
    document=disk_document(monkeypatch);node=document['seq_read'];raw=node['raw_io']
    if mutation=='missing':node.pop('raw_io')
    elif mutation=='count':raw['count']=2
    elif mutation=='length':raw['latencies_ms']=[]
    elif mutation=='bytes':raw['completed_bytes']-=1
    elif mutation=='nan':raw['latencies_ms']=[float('nan')]
    elif mutation=='negative':raw['latencies_ms']=[-1]
    elif mutation=='bool':raw['latencies_ms']=[True]
    elif mutation=='time':raw['elapsed_seconds']=0
    elif mutation=='extra_field':raw['unknown']=1
    elif mutation=='extra_operation':document['unknown']=fact(1,1)
    elif mutation=='error':node['error']='short IO'
    with pytest.raises(ValueError):extract('disk',document,LIMITS)


def test_collector_original_raw_count_and_body_budgets_fail(monkeypatch):
    document=disk_document(monkeypatch)
    with pytest.raises(ValueError):extract('disk',document,SimpleNamespace(max_samples_per_metric=1,max_samples_per_run=4000000))
    with pytest.raises(ValueError):extract('disk',disk_document(monkeypatch),SimpleNamespace(max_samples_per_metric=1000000,max_samples_per_run=2))
    with pytest.raises(ValueError):write_artifact({'raw':[1]*100},io.StringIO(),Budget(100,100,10))


def test_artifact_writer_accepts_exact_byte_limit_and_refuses_one_over():
    document={'raw':[0.000123456789],'path':'unicode-\u00e9'}
    expected=json.dumps(document,indent=2,allow_nan=False)
    size=len(expected.encode('utf-8'))
    output=io.StringIO()
    write_artifact(document,output,Budget(10,10,size))
    assert output.getvalue()==expected
    with pytest.raises(ValueError):write_artifact(document,io.StringIO(),Budget(10,10,size-1))


def storage_document(monkeypatch):
    monkeypatch.setenv('CAPSEM_STORAGE_BENCH_PATHS','/tmp')
    monkeypatch.setenv('CAPSEM_STORAGE_BENCH_SIZE_MB','1')
    monkeypatch.setenv('CAPSEM_STORAGE_IO_PROFILE_SIZE_MB','1')
    monkeypatch.setenv('CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS','3')
    node={name:fact(RAND_IO_COUNT if name.startswith('rand_') else 1,
        RAND_IO_COUNT*BLOCK_4K if name.startswith('rand_') else BLOCK_1M)
        for name in ('seq_write','seq_read_cold','seq_read_warm','rand_write_4k','rand_read_4k')}
    node['io_profile']={'random_ops':3,'size_mb':1,
        'sequential':{name:{op:fact(BLOCK_1M//size,BLOCK_1M) for op in ('write','read_cold','read_warm')}
            for name,size in [('4k',4096),('64k',65536),('1m',BLOCK_1M)]},
        'random':{name:fact(3,3*BLOCK_4K) for name in ('read_4k','write_4k_sync')}}
    return {'writable':{'/tmp':node},'rootfs':{'files_found':0,'seq_reads':[],'rand_read_4k':{'error':'no files found'}}}


def test_selected_storage_matrix_requires_real_complete_paths(monkeypatch):
    document=storage_document(monkeypatch)
    metrics,facts=extract('storage',document,LIMITS)
    assert len(metrics)==16 and len(facts)==16
    assert len(metrics['writable./tmp.io_profile.random.read_4k.latency_ms']['samples'])==3
    for change in ('missing','skipped','error','missing_profile'):
        bad=storage_document(monkeypatch)
        if change=='missing':bad['writable']={}
        elif change=='skipped':bad['writable']['/tmp']={'path':'/tmp','skipped':'not writable directory'}
        elif change=='error':bad['writable']['/tmp']['error']='permission denied'
        else:bad['writable']['/tmp']['io_profile'].pop('random')
        with pytest.raises(ValueError):extract('storage',bad,LIMITS)


def test_default_optional_paths_remain_explicit_unmeasured_skips(monkeypatch):
    document=storage_document(monkeypatch);monkeypatch.delenv('CAPSEM_STORAGE_BENCH_PATHS')
    for path in ('/root','/var/tmp','/var/log','/run'):
        document['writable'][path]={'path':path,'skipped':'not writable directory'}
    metrics,_=extract('storage',document,LIMITS)
    assert len(metrics)==16 and not any('writable./root.' in key for key in metrics)


@pytest.mark.parametrize('count', [0,-1,True,1.5])
def test_operation_reservation_rejects_invalid_count(count):
    with pytest.raises(ValueError):Operations(count)


def test_original_observations_are_bounded_before_loop_and_short_io_rejected(monkeypatch):
    with capture(Budget(2,2,10000)):
        with pytest.raises(ValueError):Operations(3)
        run=Operations(2)
        with pytest.raises(OSError):run.record(0.0,4,3)
        with pytest.raises(ValueError):run.finish(1.0)
        with pytest.raises(ValueError):Operations(1)
    from capsem_bench import storage_operations
    with capture(Budget(2,2,10000)):
        monkeypatch.setattr(storage_operations,'_random_offsets',lambda *_:pytest.fail('offset allocation before admission'))
        with pytest.raises(ValueError):storage_operations._bench_random_read_profile('unused',4096,4096,3)


def test_capture_env_requires_all_config_limits(monkeypatch):
    for name in ('CAPSEM_BENCH_MAX_SAMPLES_PER_METRIC','CAPSEM_BENCH_MAX_SAMPLES_PER_RUN','CAPSEM_BENCH_MAX_COLLECTOR_BYTES'):
        monkeypatch.delenv(name,raising=False)
    assert environment_budget() is None
    monkeypatch.setenv('CAPSEM_BENCH_MAX_SAMPLES_PER_METRIC','5')
    with pytest.raises(ValueError):environment_budget()


@pytest.mark.parametrize('operation',['disk_write','disk_read','profile_read','profile_write'])
def test_actual_operation_callers_reject_short_io(tmp_path,monkeypatch,operation):
    from capsem_bench import disk, storage_operations

    path=tmp_path/'io';path.write_bytes(b'0'*BLOCK_1M)
    monkeypatch.setattr(disk,'drop_caches',lambda:None)
    monkeypatch.setattr(storage_operations,'drop_caches',lambda:None)
    with capture(Budget(1000,1000,100000)), pytest.raises(OSError,match='short I/O'):
        if operation=='disk_write':
            monkeypatch.setattr(disk.os,'write',lambda _fd,data:len(data)-1)
            disk.bench_seq_write(str(path),BLOCK_1M)
        elif operation=='disk_read':
            monkeypatch.setattr(disk.os,'read',lambda *_:b'')
            disk.bench_seq_read(str(path),BLOCK_1M)
        elif operation=='profile_read':
            monkeypatch.setattr(storage_operations.os,'pread',lambda *_:b'abc')
            storage_operations._bench_random_read_profile(str(path),BLOCK_1M,BLOCK_4K,2)
        else:
            monkeypatch.setattr(storage_operations.os,'pwrite',lambda *_:3)
            storage_operations._bench_random_write_profile(str(path),BLOCK_1M,BLOCK_4K,2)

def test_real_small_profile_preserves_one_array_for_each_operation(tmp_path,monkeypatch):
    from capsem_bench import storage_operations
    from capsem_bench.storage import io_profile_bench

    monkeypatch.setattr(storage_operations,'drop_caches',lambda:None)
    with capture(Budget(1000,1000,100000)):
        profile=io_profile_bench(str(tmp_path),size_mb=1,seq_block_sizes=(BLOCK_4K,),rand_op_count=3)
    for operation in profile['sequential']['4k'].values():
        assert len(operation['raw_io']['latencies_ms'])==256
        assert operation['raw_io']['completed_bytes']==BLOCK_1M
    for operation in profile['random'].values():
        assert len(operation['raw_io']['latencies_ms'])==3
        assert operation['raw_io']['completed_bytes']==3*BLOCK_4K


def test_original_disk_artifact_reaches_real_collector_protocol_once(monkeypatch,capsys):
    document={'disk':disk_document(monkeypatch)}
    original=copy.deepcopy(document)
    calls=[]
    def post(path,body=None,**kwargs):
        calls.append((path,body,kwargs))
        return {'id':'vm'} if path=='/vms/create' else {'exit_code':0,'stdout':{'encoding':'utf8','data':json.dumps(document)}}
    main=runpy.run_path(str(ROOT/'benchmarks/collectors/_guest'))['main']
    monkeypatch.setitem(main.__globals__,'ServiceInstance',lambda:SimpleNamespace(
        client=lambda:SimpleNamespace(post=post,delete=lambda *_:None),start=lambda:None,stop=lambda:None))
    monkeypatch.setitem(main.__globals__,'wait_exec_ready',lambda *_args,**_kwargs:True)
    monkeypatch.setattr(sys,'argv',['disk'])
    assert main()==0
    output=json.loads(capsys.readouterr().out)
    assert output['metrics']['rand_read_4k.latency_ms']['samples']==original['disk']['rand_read_4k']['raw_io']['latencies_ms']
    assert 'latencies_ms' not in output['sidecar']
    assert json.loads(output['sidecar'])['io_capture_facts']['rand_read_4k']['count']==RAND_IO_COUNT
    command=calls[1][1]['command']
    assert 'CAPSEM_BENCH_MAX_SAMPLES_PER_METRIC=1000000' in command
    assert 'CAPSEM_BENCH_MAX_SAMPLES_PER_RUN=4000000' in command
    assert 'CAPSEM_BENCH_MAX_COLLECTOR_BYTES=67108864' in command
