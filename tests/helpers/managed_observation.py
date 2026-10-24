"""One native-owner readiness loop; strict receipts are an explicit mode."""
from __future__ import annotations

import errno
import json
import math
import os
import sys
import time
from dataclasses import dataclass, field
from typing import TYPE_CHECKING

import psutil

from .http_measurement import MeasuredResponse
from .http_transport import Transport
from .process_snapshot import OwnedProcess

if TYPE_CHECKING:
    from .managed_service import GatewayIdentity


@dataclass(frozen=True)
class ReadinessPolicy:
    deadline: float
    poll_seconds: float
    max_response_bytes: int
    max_attempts: int

    def __post_init__(self):
        if not math.isfinite(self.deadline) or not math.isfinite(self.poll_seconds) or self.poll_seconds <= 0:
            raise ValueError('readiness deadline/poll invalid')
        if any(type(value) is not int or value <= 0 for value in (self.max_response_bytes,self.max_attempts)):
            raise ValueError('readiness byte/attempt budget invalid')


@dataclass(frozen=True)
class ReadyObservation:
    identity: GatewayIdentity = field(repr=False)
    receipt: MeasuredResponse | None = field(repr=False)
    outcomes: tuple[str,...]
    ready_ns: int
    retired_ns: int
    complete_ns: int


def retired(original: tuple[OwnedProcess,...]) -> bool:
    for owned in original:
        if owned.pid != owned.process.pid or owned.started_at != owned.process.create_time():
            raise ValueError('original retirement fingerprint forged')
        try:
            if owned.process.is_running() and owned.process.status() != psutil.STATUS_ZOMBIE:
                return False
        except psutil.NoSuchProcess:
            pass
    return True


def strict_probe(identity,policy,remaining):
    client = Transport(host='127.0.0.1',port=identity.port)
    try:
        receipt = client.measure_request('GET','/vms/list',headers={'Authorization':'Bearer '+identity.token},
            timeout=remaining,max_response_bytes=policy.max_response_bytes)
    finally:
        client.close()
    first = receipt.first_body_byte_ns
    if receipt.status != 200 or receipt.attempts != 1 or first is None or not receipt.body or not 0 <= receipt.headers_ns <= first <= receipt.complete_ns:
        raise ValueError('strict readiness requires complete original HTTP200 receipt')
    value = json.loads(receipt.body)
    if not isinstance(value,dict) or not isinstance(value.get('sandboxes'),list):
        raise ValueError('readiness inventory shape invalid')
    return receipt


def wait_ready(owner,previous=None,*,policy: ReadinessPolicy | None=None,original: tuple[OwnedProcess,...]=()) -> ReadyObservation:
    from .managed_service import GatewayIdentity
    if policy is not None and previous is not None and not {previous.service_pid,previous.gateway_pid} <= {entry.pid for entry in original}:
        raise ValueError('observed restart requires original service and gateway fingerprints')
    started = time.monotonic_ns()
    deadline = time.monotonic()+45 if policy is None else policy.deadline
    interval = .1 if policy is None else policy.poll_seconds
    outcomes: list[str] = []
    first_ready = None
    ready_ns = 0
    retired_ns: int | None = None
    last_probe = 'not attempted'
    def completed(identity,receipt):
        if policy is not None and time.monotonic() > deadline:
            raise RuntimeError('strict readiness deadline expired')
        return ReadyObservation(identity,receipt,tuple(outcomes),ready_ns,retired_ns or 0,time.monotonic_ns()-started)
    while time.monotonic() < deadline:
        if policy is not None and len(outcomes) >= policy.max_attempts:
            raise RuntimeError('strict readiness attempt budget exhausted')
        elapsed = time.monotonic_ns()-started
        if retired_ns is None and retired(original):
            retired_ns = elapsed
        if first_ready is not None:
            if retired_ns is not None:
                identity,receipt = first_ready
                receipt = strict_probe(identity,policy,deadline-time.monotonic())
                outcomes.append('reconnected')
                return completed(identity,receipt)
            outcomes.append('waiting_retirement')
            time.sleep(min(interval,max(0,deadline-time.monotonic())))
            continue
        probe = 'supervisor PID'
        try:
            root = owner.service.tmp_dir
            remaining = deadline-time.monotonic()
            if remaining <= 0:
                raise RuntimeError('strict readiness deadline expired')
            pid = owner.pid() if policy is None else owner.pid(timeout=remaining)
            owner.remember(pid)
            probe = 'gateway runtime files'
            if policy is None:
                texts = [(root/name).read_text().strip() for name in ('gateway.port','gateway.token','gateway.pid')]
            else:
                texts = []
                for name in ('gateway.port','gateway.token','gateway.pid'):
                    with (root/name).open('rb') as source:
                        raw = source.read(policy.max_response_bytes+1)
                    if len(raw) > policy.max_response_bytes:
                        raise RuntimeError('readiness identity bytes exceed budget')
                    texts.append(raw.decode().strip())
            if not all(texts):
                outcomes.append('incomplete_identity')
            else:
                identity = GatewayIdentity(int(texts[0]),texts[1],pid,int(texts[2]))
                changed = previous is None or (identity.service_pid != previous.service_pid and identity.gateway_pid != previous.gateway_pid and identity.token != previous.token)
                if not changed:
                    last_probe = 'gateway identity has not rotated'
                    outcomes.append('unchanged_identity')
                else:
                    probe = 'gateway HTTP'
                    if sys.platform == 'linux':
                        namespaces = (os.readlink(f'/proc/{process}/ns/net') for process in (os.getpid(),identity.service_pid,identity.gateway_pid))
                        probe += ' test/service/gateway namespaces='+'/'.join(namespaces)
                    if policy is None:
                        status,_ = identity.get('/vms/list')
                        last_probe = f'{probe} status={status}'
                        if status == 200:
                            return ReadyObservation(identity,None,(),time.monotonic_ns()-started,retired_ns or 0,time.monotonic_ns()-started)
                    else:
                        remaining = deadline-time.monotonic()
                        if remaining <= 0:
                            raise RuntimeError('strict readiness deadline expired')
                        receipt = strict_probe(identity,policy,remaining)
                        # Runtime metadata cannot widen resource ownership to
                        # an unrelated PID (or launchd/PID1). Refresh only the
                        # authenticated service's existing descendant tree.
                        owner.remember_gateway(pid,identity.gateway_pid)
                        first_ready = identity,receipt
                        ready_ns = time.monotonic_ns()-started
                        outcomes.append('ready')
                        if retired_ns is not None:
                            return completed(identity,receipt)
        except (OSError,ValueError,ConnectionError) as error:
            if policy is not None:
                cause = error.__cause__
                if isinstance(error,FileNotFoundError):
                    outcomes.append('incomplete_identity')
                elif probe == 'supervisor PID' and isinstance(error,ValueError):
                    outcomes.append('no_pid')
                elif isinstance(cause,OSError) and cause.errno == errno.ECONNREFUSED:
                    outcomes.append('connect_refused')
                else:
                    raise RuntimeError('strict readiness probe failed') from error
            last_probe = f'{probe}: {type(error).__name__} errno={getattr(error,"errno",None)}'
        time.sleep(min(interval,max(0,deadline-time.monotonic())))
    if policy is not None:
        raise RuntimeError('strict readiness deadline expired')
    state = f'{owner.startup_state()}; last probe: {last_probe}'
    (owner.service.tmp_dir/'managed-supervisor-state.txt').write_text(state)
    raise AssertionError(f'managed gateway did not become ready; {state}; evidence: {owner.service.tmp_dir}')
