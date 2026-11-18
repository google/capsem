"""Saved workload targets are discovered once through the public HTTP facade."""
from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager

import pytest
from aiohttp import web
from capsem import Hypervisor, models

from .facade_gateway import response_model


@asynccontextmanager
async def target_gateway(has_image):
    requests = []
    managed = None

    async def handle(request):
        nonlocal managed
        assert request.headers['Authorization'] == 'Bearer token'
        body = await request.json() if request.can_read_body else None
        requests.append((request.path, body))
        ownership = {'request_id': managed['request_id'] if managed else '',
                     'generation': '22222222-2222-4222-8222-222222222222',
                     'expires_at': '2099-01-01T00:00:00Z', 'lease_seconds': 4, 'renew_after_seconds': 1}
        if request.path == '/vms/create':
            managed = body.get('managed')
            ownership['request_id'] = managed['request_id'] if managed else ''
            return web.json_response(response_model('ProvisionResponse', id='vm-1', name='vm-1',
                                                    persistent=False, ownership=ownership if managed else None))
        if request.path.endswith('/claim'):
            return web.json_response({'ownership': ownership, 'state': 'active'})
        if request.path.endswith('/close'):
            return web.json_response({'request_id': managed['request_id'], 'generation': ownership['generation'],
                                      'state': 'closed', 'session_id': 'vm-1', 'evidence_retained': False})
        if request.path.endswith('/container'):
            if not has_image:
                return web.Response(status=404, text='no container')
            return web.json_response(response_model('ContainerStatusResponse'))
        assert request.path.endswith('/exposures')
        return web.json_response(response_model('ExposureInfo', **{**body, 'id': 'port', 'host_port': 49152}))

    app = web.Application()
    app.router.add_route('*', '/{tail:.*}', handle)
    runner = web.AppRunner(app, shutdown_timeout=0.2)
    await runner.setup()
    try:
        await web.TCPSite(runner, '127.0.0.1', 0).start()
        yield f'http://127.0.0.1:{runner.addresses[0][1]}', requests
    finally:
        await runner.cleanup()


@pytest.mark.parametrize('managed', [False, True])
@pytest.mark.parametrize('has_image,attached,inline', [(True, False, False), (False, False, False),
                                                     (True, True, False), (True, False, True)])
def test_saved_environment_ports_discover_target_once(managed, has_image, attached, inline):
    async def run():
        reference = models.EnvironmentRevisionReference(
            environment_id='00000000-0000-4000-8000-000000000001',
            revision_id='00000000-0000-4000-8000-000000000002')
        options = {'image': 'busybox'} if inline else {'environment': reference, 'attach': attached}
        async with target_gateway(has_image) as (url, requests), Hypervisor(url, 'token') as hv:
            async def ports(vm):
                await vm.ports.open(8080)
                await vm.ports.open(8081)
            if managed:
                async with hv.managed(**options) as vm:
                    await ports(vm)
            else:
                vm = await hv.create(**options)
                await ports(vm)
                await vm.close()
            assert sum(path.endswith('/container') for path, _ in requests) == int(not attached and not inline)
            exposures = [body for path, body in requests if path.endswith('/exposures')]
            assert len(exposures) == 2
            assert all(body['target'] == ('container' if has_image else 'vm') for body in exposures)
    asyncio.run(run())
