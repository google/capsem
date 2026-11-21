"""Saved workload targets are discovered once through the public HTTP facade."""
from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from typing import TypedDict

import pytest
from aiohttp import web
from capsem import HttpError, Hypervisor, models

from .facade_gateway import response_model


class LaunchOptions(TypedDict, total=False):
    image: str
    environment: models.EnvironmentRevisionReference
    attach: bool


@asynccontextmanager
async def target_gateway(has_image, *, startup_seconds=600, metadata="verified", timing_entered=None, timing_release=None):
    requests = []
    managed = None

    async def handle(request):
        nonlocal managed
        assert request.headers['Authorization'] == 'Bearer token'
        body = await request.json() if request.can_read_body else None
        requests.append((request.path, body))
        if request.path.endswith('/readiness'):
            if timing_entered is not None:
                timing_entered.set()
                assert timing_release is not None
                await timing_release.wait()
            if metadata == "unavailable":
                return web.Response(status=403, text="inspection refused")
            parts = request.path.split('/')
            return web.json_response({'environment': {'environment_id': parts[2],
                'revision_id': '00000000-0000-4000-8000-000000000009' if metadata == "changed" else parts[4]},
                'startup_timeout_seconds': 2**32 if metadata == 'overflow' else startup_seconds if has_image else 0})
        ownership = {'request_id': managed['request_id'] if managed else '',
                     'generation': '22222222-2222-4222-8222-222222222222',
                     'expires_at': '2099-01-01T00:00:00Z', 'lease_seconds': 4, 'renew_after_seconds': 1}
        if request.path == '/vms/create':
            assert body is not None
            managed = body.get('managed')
            ownership['request_id'] = managed['request_id'] if managed else ''
            return web.json_response(response_model('ProvisionResponse', id='vm-1', name='vm-1',
                                                    persistent=False, ownership=ownership if managed else None))
        if request.path.endswith('/claim'):
            return web.json_response({'ownership': ownership, 'state': 'active'})
        if request.path.endswith('/close'):
            assert managed is not None
            return web.json_response({'request_id': managed['request_id'], 'generation': ownership['generation'],
                                      'state': 'closed', 'session_id': 'vm-1', 'evidence_retained': False})
        if request.path.endswith('/container'):
            if not has_image:
                return web.Response(status=404, text='no container')
            return web.json_response(response_model('ContainerStatusResponse'))
        assert request.path.endswith('/exposures')
        assert body is not None
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
        options: LaunchOptions = {'image': 'busybox'} if inline else {'environment': reference, 'attach': attached}
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
            assert sum(path.endswith('/readiness') for path, _ in requests) == int(not attached and not inline)
            assert sum(path.endswith('/container') for path, _ in requests) == int(not attached and not inline)
            exposures = [body for path, body in requests if path.endswith('/exposures')]
            assert len(exposures) == 2
            assert all(body['target'] == ('container' if has_image else 'vm') for body in exposures)
    asyncio.run(run())


@pytest.mark.parametrize('managed', [False, True])
@pytest.mark.parametrize('metadata', ['changed', 'unavailable', 'overflow'])
def test_timing_refusal_precedes_create_and_managed_ownership(managed, metadata):
    async def run():
        reference = models.EnvironmentRevisionReference(
            environment_id='00000000-0000-4000-8000-000000000001',
            revision_id='00000000-0000-4000-8000-000000000002')
        async with target_gateway(True, metadata=metadata) as (url, requests), Hypervisor(url, 'token') as hv:
            with pytest.raises((ValueError, HttpError)):
                if managed:
                    async with hv.managed(environment=reference):
                        pytest.fail('unverified timing must not enter the managed body')
                else:
                    await hv.create(environment=reference)
            assert len(requests) == 1 and requests[0][0].endswith('/readiness')
    asyncio.run(run())


@pytest.mark.parametrize('seconds', [0, 600, 2**32 - 1])
def test_creation_wait_keeps_the_exact_approved_deadline(seconds):
    from capsem._transport import Transport
    from capsem.readiness import prepare_creation

    async def run():
        selected = models.EnvironmentRevisionReference(
            environment_id='00000000-0000-4000-8000-000000000001',
            revision_id='00000000-0000-4000-8000-000000000002')
        async with target_gateway(True, startup_seconds=seconds) as (url, requests):
            async with Transport(url, 'token') as transport:
                assert await prepare_creation(transport, models.ProvisionRequest(environment=selected)) == 230 + seconds
            async with Transport(url, 'token', timeout=10_000_000_000) as transport:
                assert await prepare_creation(transport, models.ProvisionRequest(environment=selected)) == 10_000_000_000
            assert len(requests) == 2 and all(path.endswith('/readiness') for path, _ in requests)
    asyncio.run(run())


def test_cancelling_managed_timing_read_never_admits_creation_or_cleanup():
    async def run():
        entered, release = asyncio.Event(), asyncio.Event()
        selected = models.EnvironmentRevisionReference(
            environment_id='00000000-0000-4000-8000-000000000001',
            revision_id='00000000-0000-4000-8000-000000000002')
        async with target_gateway(True, timing_entered=entered, timing_release=release) as (url, requests), Hypervisor(url, 'token') as hv:
            scope = hv.managed(environment=selected)
            task = asyncio.create_task(scope.__aenter__())
            try:
                await asyncio.wait_for(entered.wait(), 2)
                task.cancel()
                with pytest.raises(asyncio.CancelledError):
                    await task
                assert len(requests) == 1 and requests[0][0].endswith('/readiness')
                with pytest.raises(RuntimeError, match='cannot be entered again'):
                    await scope.__aenter__()
            finally:
                release.set()
                if not task.done():
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)
    asyncio.run(run())
