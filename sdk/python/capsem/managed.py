"""Finite managed ownership, independent of ordinary VM connection lifetime."""

from __future__ import annotations

import asyncio
from datetime import UTC, datetime
from secrets import token_hex
from types import TracebackType
from uuid import UUID, uuid4

from . import _operations as api
from . import models
from ._transport import HttpError, Transport
from .execution import CREATE_READY_SECS, GATEWAY_REQUEST_BUDGET_SECS
from .vm import VM


class ManagedSession:
    """Use ``async with hv.managed(...) as vm`` for an owned ephemeral VM.

    One create is issued. Connection loss, cancellation and close never replay
    it. A client crash stops renewal; the service owns finite lease expiry.
    """

    def __init__(self, transport: Transport, request: models.ProvisionRequest) -> None:
        if request.name is not None or request.persistent or request.managed is not None:
            raise ValueError("managed sessions require an unnamed ephemeral create")
        self.__transport = transport
        self.__request_id = str(uuid4())
        self.__token = token_hex(32)
        self.__request = request
        self.__request.managed = models.ManagedSessionCreate(
            request_id=self.__request_id, ownership_token=self.__token)
        self.__timeout = max(transport.timeout, CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS)
        self.__generation: str | None = None
        self.__ownership: models.ManagedSessionOwnership | None = None
        self.__vm: VM | None = None
        self.__vm_id: str | None = None
        self.__started = False
        self.__active = False
        self.__finished = False
        self.__create: asyncio.Task[models.ProvisionResponse] | None = None
        self.__renew: asyncio.Task[None] | None = None
        self.__closing: asyncio.Task[None] | None = None
        self.__caller: asyncio.Task | None = None
        self.__failure: Exception | None = None
        self.__changed = asyncio.Event()

    @property
    def ownership(self) -> models.ManagedSessionOwnership | None:
        """Verified nonsecret metadata; the capability is never exposed here."""
        return self.__ownership.model_copy() if self.__ownership is not None else None

    def __identity(self, request_id: str, generation: str) -> None:
        try:
            parsed = UUID(generation)
        except ValueError:
            raise RuntimeError("invalid managed ownership identity") from None
        if (request_id != self.__request_id or parsed.version != 4 or str(parsed) != generation
                or (self.__generation is not None and self.__generation != generation)):
            raise RuntimeError("managed ownership changed")
        self.__generation = generation

    def __adopt(self, ownership: models.ManagedSessionOwnership) -> None:
        self.__identity(ownership.request_id, ownership.generation)
        if not 0 < ownership.renew_after_seconds < ownership.lease_seconds:
            raise RuntimeError("invalid managed ownership lease")
        try:
            expiry = datetime.fromisoformat(ownership.expires_at.replace("Z", "+00:00"))
            if expiry.tzinfo is None or expiry <= datetime.now(UTC):
                raise ValueError
        except ValueError:
            raise RuntimeError("managed ownership lease expired or invalid") from None
        self.__ownership = ownership
        self.__changed.set()

    def __session(self, response: models.ProvisionResponse) -> None:
        if response.ownership is None or response.persistent or not response.id:
            raise RuntimeError("managed session omitted its ephemeral ownership")
        self.__adopt(response.ownership)
        if self.__vm_id is not None and self.__vm_id != response.id:
            raise RuntimeError("managed ownership changed VM identity")
        self.__vm_id = response.id

    async def __aenter__(self) -> VM:
        if self.__started or self.__finished:
            raise RuntimeError("managed ownership scope cannot be entered again")
        self.__started = True
        self.__caller = asyncio.current_task()
        self.__create = asyncio.create_task(api.create_vm(
            self.__transport, body=self.__request, request_timeout=self.__timeout))
        self.__renew = asyncio.create_task(self.__maintain())
        try:
            await asyncio.wait((self.__create, self.__renew), return_when=asyncio.FIRST_COMPLETED)
            if self.__renew.done():
                await self.__renew
            response = await self.__create
            self.__session(response)
            self.__active = True
            self.__vm = VM._bind(self.__transport, id=response.id, name=response.name,
                                 container=self.__request.container is not None)
            return self.__vm
        except BaseException as error:
            outcome = models.ManagedSessionOutcome.CANCELLED if isinstance(error, asyncio.CancelledError) else models.ManagedSessionOutcome.FAILED
            await self.__join_close(outcome, retry=False)
            raise

    async def __aexit__(self, _kind: type[BaseException] | None, error: BaseException | None,
                        _traceback: TracebackType | None) -> None:
        outcome = (models.ManagedSessionOutcome.FAILED if self.__failure is not None
                   else models.ManagedSessionOutcome.CANCELLED if isinstance(error, asyncio.CancelledError)
                   else models.ManagedSessionOutcome.FAILED if error is not None
                   else models.ManagedSessionOutcome.COMPLETED)
        await self.__join_close(outcome, retry=False)
        if self.__failure is not None:
            raise self.__failure

    async def __maintain(self) -> None:
        try:
            while True:
                self.__changed.clear()
                ownership = self.__ownership
                delay = 0.05 if ownership is None else float(ownership.renew_after_seconds)
                try:
                    await asyncio.wait_for(self.__changed.wait(), delay)
                    continue
                except TimeoutError:
                    pass
                try:
                    claim = await api.claim_managed_session(self.__transport, request_id=self.__request_id,
                        body=models.ManagedSessionClaimRequest(ownership_token=self.__token))
                except HttpError as error:
                    if error.status == 404 and self.__generation is None and self.__create is not None and not self.__create.done():
                        continue  # Admission can race the first safe claim; never repeat create.
                    raise
                if claim.state not in (models.ManagedSessionState.PENDING, models.ManagedSessionState.ACTIVE):
                    raise RuntimeError("managed ownership is no longer live")
                self.__adopt(claim.ownership)
                if claim.session is not None:
                    self.__session(claim.session)
        except Exception as error:
            self.__failure = error
            if self.__active and self.__caller is not None:
                if self.__closing is None:
                    self.__closing = asyncio.create_task(self.__cleanup(models.ManagedSessionOutcome.FAILED))
                self.__caller.cancel()
            raise

    async def close(self, outcome: models.ManagedSessionOutcome = models.ManagedSessionOutcome.COMPLETED) -> None:
        """Join cleanup, including repeated cancellation; a failed close may be retried explicitly."""
        await self.__join_close(outcome, retry=True)

    async def __join_close(self, outcome: models.ManagedSessionOutcome, *, retry: bool) -> None:
        if self.__finished:
            return
        if not self.__started:
            self.__finished = True
            return
        if self.__closing is None or (retry and self.__closing.done() and self.__closing.exception() is not None):
            self.__closing = asyncio.create_task(self.__cleanup(outcome))
        cancelled = False
        while True:
            try:
                await asyncio.shield(self.__closing)
                break
            except asyncio.CancelledError:
                if self.__closing.cancelled():
                    raise
                cancelled = True
        if cancelled:
            raise asyncio.CancelledError

    async def __cleanup(self, outcome: models.ManagedSessionOutcome) -> None:
        self.__active = False
        tasks = [task for task in (self.__create, self.__renew) if task is not None]
        for task in tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        if self.__vm is not None:
            await self.__vm.close()
        closed = await api.close_managed_session(self.__transport, request_id=self.__request_id,
            body=models.ManagedSessionCloseRequest(ownership_token=self.__token, outcome=outcome),
            request_timeout=self.__timeout)
        self.__identity(closed.request_id, closed.generation)
        if self.__vm_id is not None and closed.session_id != self.__vm_id:
            raise RuntimeError("managed close acknowledged another VM identity")
        self.__finished = True
        self.__caller = None
        self.__request.managed = None
        self.__token = ""
