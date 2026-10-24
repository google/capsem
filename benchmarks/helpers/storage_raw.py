"""Strict original I/O arrays from the existing bounded guest JSON artifact.

The entry point imports the same guest declaration constants, never another
workload or percentile implementation. Raw arrays move out before sidecar output.
"""
import math
import os

from capsem_bench.helpers import BLOCK_1M, BLOCK_4K, DEFAULT_SIZE_MB, RAND_IO_COUNT
from capsem_bench.storage import (
    DEFAULT_IO_PROFILE_RANDOM_OPS,
    DEFAULT_IO_PROFILE_SIZE_MB,
    DEFAULT_STORAGE_PATHS,
    DEFAULT_STORAGE_SIZE_MB,
    IO_PROFILE_BLOCK_SIZES,
    ROOTFS_RAND_COUNT,
)


def integer(value, name):
    if type(value) is not int or value <= 0:
        raise ValueError('invalid I/O '+name)
    return value


def positive_env(name, default):
    return integer(int(os.environ.get(name,str(default))),name)


def extract(mode, section, limits):
    metrics, facts = {}, {}
    total = 0

    def record(node, key, count, completed_bytes=None):
        nonlocal total
        if not isinstance(node,dict) or 'error' in node or 'skipped' in node:
            raise ValueError('required I/O operation failed: '+key)
        raw = node.pop('raw_io',None)
        if not isinstance(raw,dict) or set(raw) != {'latencies_ms','count','completed_bytes','elapsed_seconds'}:
            raise ValueError('missing original I/O facts: '+key)
        observed = raw.pop('latencies_ms')
        if type(raw['count']) is not int or raw['count'] != count:
            raise ValueError('I/O count differs from selected workload: '+key)
        if not isinstance(observed,list) or len(observed)!=count or count>limits.max_samples_per_metric:
            raise ValueError('missing/overbudget original I/O samples: '+key)
        if any(type(x) not in (int,float) or not math.isfinite(x) or x<0 for x in observed):
            raise ValueError('invalid original I/O sample: '+key)
        size=integer(raw['completed_bytes'],'completed bytes')
        if completed_bytes is not None and size!=completed_bytes:
            raise ValueError('incomplete original I/O bytes: '+key)
        elapsed=raw['elapsed_seconds']
        if type(elapsed) not in (int,float) or not math.isfinite(elapsed) or elapsed<=0:
            raise ValueError('invalid original I/O clock: '+key)
        total += count
        if total>limits.max_samples_per_run:
            raise ValueError('total I/O sample budget exceeded')
        metrics[key+'.latency_ms']={'unit':'milliseconds','samples':observed}
        facts[key]=raw

    def disk(node, prefix, size):
        for name in ('seq_write','seq_read','rand_write_4k','rand_read_4k'):
            random=name.startswith('rand_')
            count=RAND_IO_COUNT if random else size//BLOCK_1M
            record(node.get(name),prefix+name,count,count*(BLOCK_4K if random else BLOCK_1M))

    if mode=='disk':
        disk(section,'',positive_env('CAPSEM_BENCH_SIZE_MB',DEFAULT_SIZE_MB)*BLOCK_1M)
    elif mode=='storage':
        explicit=os.environ.get('CAPSEM_STORAGE_BENCH_PATHS')
        paths=list(dict.fromkeys(p.strip() for p in explicit.split(':') if p.strip())) if explicit is not None else DEFAULT_STORAGE_PATHS
        if not paths or set(section.get('writable',{}))!=set(paths):
            raise ValueError('storage path matrix differs from selected workload')
        successes=0
        size=positive_env('CAPSEM_STORAGE_BENCH_SIZE_MB',DEFAULT_STORAGE_SIZE_MB)*BLOCK_1M
        profile_size=positive_env('CAPSEM_STORAGE_IO_PROFILE_SIZE_MB',DEFAULT_IO_PROFILE_SIZE_MB)*BLOCK_1M
        random_count=positive_env('CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS',DEFAULT_IO_PROFILE_RANDOM_OPS)
        for path in paths:
            node=section['writable'][path]
            if node.get('skipped')=='not writable directory' and explicit is None:
                if set(node)!={'path','skipped'} or node['path']!=path:
                    raise ValueError('optional storage skip has unexpected contents')
                continue
            if 'error' in node or 'skipped' in node:
                raise ValueError('selected storage path failed: '+path)
            prefix='writable.'+path+'.'
            for name in ('seq_write','seq_read_cold','seq_read_warm','rand_write_4k','rand_read_4k'):
                random=name.startswith('rand_')
                count=RAND_IO_COUNT if random else size//BLOCK_1M
                record(node.get(name),prefix+name,count,count*(BLOCK_4K if random else BLOCK_1M))
            profile=node.get('io_profile',{})
            if profile.get('random_ops')!=random_count or profile.get('size_mb')!=profile_size//BLOCK_1M:
                raise ValueError('storage profile differs from selected workload')
            block_names=('4k','64k','1m')
            if set(profile.get('sequential',{}))!=set(block_names) or set(profile.get('random',{}))!={'read_4k','write_4k_sync'}:
                raise ValueError('incomplete storage profile matrix')
            for label,block in zip(block_names,IO_PROFILE_BLOCK_SIZES,strict=True):
                if set(profile['sequential'][label])!={'write','read_cold','read_warm'}:
                    raise ValueError('incomplete sequential I/O matrix')
                for operation,node in profile['sequential'][label].items():
                    record(node,prefix+'io_profile.sequential.'+label+'.'+operation,profile_size//block,profile_size)
            for operation,node in profile['random'].items():
                record(node,prefix+'io_profile.random.'+operation,random_count,random_count*BLOCK_4K)
            successes += 1
        if not successes:
            raise ValueError('no successful writable storage measurement')
        rootfs=section.get('rootfs',{})
        if type(rootfs.get('files_found')) is not int or rootfs['files_found']<0 or not isinstance(rootfs.get('seq_reads'),list):
            raise ValueError('missing or invalid optional rootfs inventory')
        for index,node in enumerate(rootfs.get('seq_reads',[])):
            size=integer(node.get('size_bytes'),'rootfs selected size')
            for kind in ('cold','warm'):
                record(node.get(kind),f'rootfs.seq_reads.{index}.{kind}',(size+BLOCK_1M-1)//BLOCK_1M,size)
        if rootfs.get('files_found',0):
            node=rootfs.get('rand_read_4k',{})
            record(node,'rootfs.rand_read_4k',ROOTFS_RAND_COUNT)
            if facts['rootfs.rand_read_4k']['completed_bytes']>ROOTFS_RAND_COUNT*BLOCK_4K:
                raise ValueError('rootfs I/O byte count exceeds selected operations')
        elif rootfs.get('rand_read_4k',{}).get('error')!='no files found':
            raise ValueError('missing explicit optional rootfs measurement result')
    else:
        raise ValueError('raw I/O mode unsupported')
    def reject_unconsumed(node):
        if isinstance(node,dict):
            if 'raw_io' in node:
                raise ValueError('unbound extra I/O operation in artifact')
            for value in node.values():
                reject_unconsumed(value)
        elif isinstance(node,list):
            for value in node:
                reject_unconsumed(value)
    reject_unconsumed(section)
    return metrics,facts
