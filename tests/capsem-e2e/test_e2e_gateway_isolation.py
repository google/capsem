"""Separate CLI service instances never compete for the same gateway port."""

import pytest

pytestmark = pytest.mark.e2e


def test_two_real_services_have_independent_gateways(service):
    second = type(service)()
    try:
        second.start()
        service.cli_ok("list")
        second.cli_ok("list")
        first_port = int((service.tmp_dir / "gateway.port").read_text())
        second_port = int((second.tmp_dir / "gateway.port").read_text())
        assert first_port != second_port
    finally:
        second.stop()
