"""Bounded original I/O observations in the existing diagnostic artifact.

Only the host collector supplies the existing benchmark.run limits. Human
summaries stay diagnostics; this module computes no distribution statistics.
"""
import contextlib
import contextvars
import json
import math
import os
import time

LIMIT_ENV = (
    'CAPSEM_BENCH_MAX_SAMPLES_PER_METRIC',
    'CAPSEM_BENCH_MAX_SAMPLES_PER_RUN',
    'CAPSEM_BENCH_MAX_COLLECTOR_BYTES',
)
_CURRENT = contextvars.ContextVar('io_capture', default=None)


class Budget:
    def __init__(self, per_metric, total, output_bytes):
        if any(type(x) is not int or x <= 0 for x in (per_metric, total, output_bytes)):
            raise ValueError('I/O capture limits must be positive integers')
        self.per_metric, self.total, self.output_bytes = per_metric, total, output_bytes
        self.reserved = 0

    def reserve(self, count):
        if type(count) is not int or not 0 < count <= self.per_metric or self.reserved+count > self.total:
            raise ValueError('I/O original sample count exceeds configured budget')
        self.reserved += count


@contextlib.contextmanager
def capture(budget):
    token = _CURRENT.set(budget)
    try:
        yield
    finally:
        _CURRENT.reset(token)


def environment_budget():
    values = [os.environ.get(name) for name in LIMIT_ENV]
    if not any(value is not None for value in values):
        return None
    if not all(value is not None and value.isdecimal() for value in values):
        raise ValueError('incomplete or invalid collector I/O limits')
    per_metric, total, output_bytes = values
    assert isinstance(per_metric,str) and isinstance(total,str) and isinstance(output_bytes,str)
    return Budget(int(per_metric),int(total),int(output_bytes))


class Operations:
    def __init__(self, count, *, human_latencies=False):
        if type(count) is not int or count <= 0:
            raise ValueError('I/O operation count must be positive')
        self.budget = _CURRENT.get()
        if self.budget is not None:
            self.budget.reserve(count)
        self.expected, self.count, self.completed_bytes = count, 0, 0
        self.latencies = [] if self.budget is not None or human_latencies else None

    def record(self, started, requested, completed):
        if type(requested) is not int or requested <= 0 or type(completed) is not int or completed != requested:
            raise OSError('short I/O operation; incomplete bytes cannot be benchmarked')
        self.count += 1
        if self.count > self.expected:
            raise ValueError('unexpected extra I/O operation')
        self.completed_bytes += completed
        if self.latencies is not None:
            latency = (time.monotonic()-started)*1000
            if not math.isfinite(latency) or latency < 0:
                raise ValueError('invalid original I/O latency')
            self.latencies.append(latency)

    def finish(self, elapsed):
        if self.count != self.expected or type(elapsed) not in (int,float) or not math.isfinite(elapsed) or elapsed <= 0:
            raise ValueError('incomplete I/O count or invalid original elapsed time')
        if self.budget is None:
            return {}
        return {'raw_io': {'latencies_ms': self.latencies, 'count': self.count,
            'completed_bytes': self.completed_bytes, 'elapsed_seconds': elapsed}}


def raw_summary(size_bytes, count, elapsed, latencies):
    """Retain an already observed array once; used by existing profile callers."""
    budget = _CURRENT.get()
    if budget is None:
        return None
    if type(count) is not int or not 0 < count <= budget.per_metric or len(latencies) != count:
        raise ValueError('original I/O sample count differs')
    if type(size_bytes) is not int or size_bytes <= 0 or type(elapsed) not in (int,float) or not math.isfinite(elapsed) or elapsed <= 0:
        raise ValueError('invalid original I/O byte/time facts')
    if any(type(x) not in (int,float) or not math.isfinite(x) or x < 0 for x in latencies):
        raise ValueError('invalid original I/O latency')
    return {'latencies_ms':latencies,'count':count,'completed_bytes':size_bytes,'elapsed_seconds':elapsed}


def write_artifact(document, output, budget):
    """Same JSON artifact writer, bounded during encoding rather than after it."""
    class Writer:
        def __init__(self):
            self.written = 0

        def write(self, text):
            self.written += len(text.encode('utf-8'))
            if budget is not None and self.written > budget.output_bytes:
                raise ValueError('benchmark artifact exceeds configured byte budget')
            return output.write(text)

    json.dump(document, Writer(), indent=2, allow_nan=False)
