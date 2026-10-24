"""Peer retirement belongs to the existing original HTTP transport socket."""
import pytest
from helpers.http_transport import Transport

from tests.test_http_measurement import wire

REPLY = b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok'


@pytest.mark.parametrize('reset',[False,True])
def test_actual_original_peer_eof_or_reset_never_reconnects(reset):
    with wire([(0,REPLY)],reset=reset) as (transport,seen):
        assert transport.measure_request('GET','/',timeout=1,max_response_bytes=2).body == b'ok'
        transport.expect_peer_close(timeout=.5)
        assert len(seen) == 1 and transport._connections == {}


def test_original_open_peer_timeout_is_not_a_success():
    with wire([(0,REPLY)],requests=2) as (transport,seen):
        assert transport.measure_request('GET','/',timeout=1,max_response_bytes=2).body == b'ok'
        with pytest.raises(TimeoutError):
            transport.expect_peer_close(timeout=.01)
        assert len(seen) == 1 and transport._connections == {}


def test_original_extra_data_is_not_a_success():
    with wire([(0,REPLY),(.01,b'unexpected')]) as (transport,seen):
        assert transport.measure_request('GET','/',timeout=1,max_response_bytes=2).body == b'ok'
        with pytest.raises(AssertionError,match='data'):
            transport.expect_peer_close(timeout=.5)
        assert len(seen) == 1 and transport._connections == {}


def test_no_original_socket_or_invalid_budget_never_creates_one():
    transport = Transport(host='127.0.0.1',port=1)
    with pytest.raises(AssertionError,match='original'):
        transport.expect_peer_close(timeout=.1)
    for timeout in (0,-1,float('inf'),float('nan')):
        with pytest.raises(ValueError):
            transport.expect_peer_close(timeout=timeout)
    assert transport._connections == {}
