"""Shared original stream owner enforces caller deadlines and finite probes."""
from types import SimpleNamespace
from typing import cast

import msgpack
import pytest
from websockets.sync.client import ClientConnection

from benchmarks.helpers import stream_protocol as subject


class Socket:
    def __init__(self,frames):
        self.frames,self.sent,self.closed = iter(frames),[],False
    def __enter__(self):
        return self
    def __exit__(self,*args):
        self.closed = True
    def send(self,value):
        self.sent.append(value)
    def recv(self,timeout):
        assert timeout > 0
        return next(self.frames)


def test_bounded_original_open_and_cleanup(monkeypatch):
    options = []
    socket = Socket([msgpack.packb({'status':{'type':'started'}})])
    monkeypatch.setattr(subject,'connect',lambda url,**kwargs:options.append((url,kwargs)) or socket)
    monkeypatch.setattr(subject,'time',SimpleNamespace(monotonic=lambda:10.))
    with subject.open_stream(subject.Gateway(123,'synthetic',456),'original',
        {'type':'start','kind':'terminal'},deadline=12.,close_timeout=1.) as stream:
        assert stream is socket
    assert socket.closed and options[0][1]['open_timeout'] == 2
    assert options[0][1]['close_timeout'] == 1
    assert len(socket.sent) == 2
    with pytest.raises(TimeoutError), subject.open_stream(subject.Gateway(123,'synthetic',456),'original',{},deadline=9.):
        raise AssertionError('expired open admitted')
    assert len(options) == 1


@pytest.mark.parametrize('limits',[{'max_frames':1},{'max_bytes':1}])
def test_probe_budget_fails_even_if_marker_arrives_later(limits):
    socket = Socket([msgpack.packb({'stdout':b'noise'}),msgpack.packb({'stdout':b'marker'})])
    with pytest.raises(ValueError,match='budget'):
        subject.read_until(cast(ClientConnection,socket),b'marker',1,**limits)


def test_split_marker_preserves_existing_frame_semantics():
    socket = Socket([msgpack.packb({'stdout':b'do'}),msgpack.packb({'stdout':b'ne'})])
    assert subject.read_until(cast(ClientConnection,socket),b'done',1,max_frames=2,max_bytes=4) == 4


@pytest.mark.parametrize('kwargs',[{'max_frames':0},{'max_bytes':True},{'max_bytes':-1}])
def test_invalid_probe_bounds_fail_before_receive(kwargs):
    with pytest.raises(ValueError,match='budget'):
        subject.read_until(cast(ClientConnection,None),b'marker',1,**kwargs)


def test_nonfinite_open_deadline_never_opens_a_socket():
    with pytest.raises(ValueError,match='finite'), subject.open_stream(subject.Gateway(123,'synthetic',456),'original',{},deadline=float('nan')):
        raise AssertionError('nonfinite open admitted')


def test_expired_readiness_never_types_into_terminal():
    with pytest.raises(TimeoutError):
        subject.ready_terminal(cast(ClientConnection,None),deadline=0)
