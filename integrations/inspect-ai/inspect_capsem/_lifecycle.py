"""Process-owned VM registry, teardown, and leftover VM sweep helpers."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import shlex
import uuid
from collections.abc import Callable
from typing import TYPE_CHECKING

from capsem import models

from inspect_capsem._controller import (
    _NONCE_LABEL,
    _TASK_LABEL,
    _is_managed_vm,
    _managed_vm_prefix_slug,
)

if TYPE_CHECKING:
    from inspect_capsem._controller import CapsemController
    from inspect_capsem.config import CapsemSandboxConfig

logger = logging.getLogger(__name__)

_INIT_TEARDOWN_TIMEOUT_SECS = 25.0
_TERMINAL_VM_STATES = frozenset(
    {
        models.VmLifecycleState.STOPPED,
        models.VmLifecycleState.DEFUNCT,
        models.VmLifecycleState.INCOMPATIBLE,
    }
)
_PROCESS_OWNED_VMS: dict[str, str | None] = {}


def _register_process_owned_vm(vm_id: str, task_name: str | None = None) -> None:
    if vm_id:
        _PROCESS_OWNED_VMS[vm_id] = task_name


def _unregister_process_owned_vm(vm_id: str) -> None:
    if vm_id:
        _PROCESS_OWNED_VMS.pop(vm_id, None)


def _abandon_process_owned_vms(task_name: str | None = None) -> list[str]:
    """Disarm process-owned tracking for task VMs so atexit will not sweep them."""
    surviving_ids = [
        vid
        for vid, rec in list(_PROCESS_OWNED_VMS.items())
        if task_name is None or rec == task_name
    ]
    for vid in surviving_ids:
        _PROCESS_OWNED_VMS.pop(vid, None)
    return surviving_ids


def _oci_create_command(cfg: CapsemSandboxConfig) -> tuple[str, ...] | None:
    if cfg.command is None:
        return None
    if isinstance(cfg.command, str):
        return tuple(shlex.split(cfg.command)) if cfg.command.strip() else None
    return tuple(cfg.command)


async def _init_sample_vm(
    controller: CapsemController, cfg: CapsemSandboxConfig, task_name: str
) -> str:
    """Start and register a VM for sample_init, cleaning up on cancellation."""
    nonce = uuid.uuid4().hex
    sample_labels = {_NONCE_LABEL: nonce}
    if task_name:
        sample_labels[_TASK_LABEL] = task_name[:255]
    oci_image = cfg.image if cfg.execution_mode == "container" else None
    ca_pem: str | None = None
    if cfg.execution_mode == "container" and cfg.build:
        from inspect_capsem.containers.image_build import build_and_stage_oci_image

        oci_image, ca_pem = await build_and_stage_oci_image(cfg.build)
    oci_cmd = _oci_create_command(cfg) if cfg.execution_mode == "container" else None
    extra_kw = {"registry_ca_pem": ca_pem} if ca_pem else {}
    start_task = asyncio.ensure_future(
        controller.start_vm(
            cpu_count=cfg.cpu_count,
            ram_gb=cfg.ram_gb,
            image=oci_image,
            command=oci_cmd,
            env=dict(cfg.environment) if cfg.environment else None,
            labels=sample_labels,
            **extra_kw,
        )
    )
    try:
        vm_id = await asyncio.shield(start_task)
    except asyncio.CancelledError:
        vm_id = ""
        with contextlib.suppress(BaseException):
            vm_id = await asyncio.wait_for(
                asyncio.shield(start_task), timeout=_INIT_TEARDOWN_TIMEOUT_SECS
            )
        if vm_id:
            try:
                await controller.stop_vm(vm_id)
            except Exception as exc:
                logger.warning(
                    "Failed to stop Capsem VM %s after cancelled start_vm: %s", vm_id, exc
                )
        else:
            start_task.cancel()
            await _sweep_failed_start_vms(controller, nonce)
        raise
    except Exception:
        await _sweep_failed_start_vms(controller, nonce)
        raise
    _register_process_owned_vm(vm_id, task_name=task_name)
    return vm_id


async def _sweep_failed_start_vms(controller: CapsemController, nonce: str) -> None:
    prefix, owned = _managed_vm_prefix_slug(), set(_PROCESS_OWNED_VMS.keys())
    with contextlib.suppress(BaseException):
        for m in await controller.list_vms():
            if (
                m.id
                and m.id not in owned
                and _is_managed_vm(m, prefix)
                and (m.labels or {}).get(_NONCE_LABEL) == nonce
            ):
                with contextlib.suppress(BaseException):
                    await controller.stop_vm(m.id)


async def _stop_vms(controller: CapsemController, vm_ids: list[str], *, reason: str) -> list[str]:
    swept: list[str] = []
    for vid in vm_ids:
        try:
            await controller.stop_vm(vid)
            _unregister_process_owned_vm(vid)
            swept.append(vid)
        except Exception as exc:
            logger.warning(
                "Failed to stop %s Capsem VM %s "
                "(run 'inspect sandbox cleanup capsem' to retry): %s",
                reason,
                vid,
                exc,
            )
    if swept:
        logger.info("Stopped %d %s Capsem VM(s): %s", len(swept), reason, swept)
    return swept


async def sweep_process_owned_vms(
    *,
    controller_factory: Callable[[], CapsemController],
    task_name: str | None = None,
    controller: CapsemController | None = None,
) -> list[str]:
    """Stop any VMs started by this process (or by `task_name`) that have not yet been stopped."""
    matching_ids = [
        vid for vid, rec in _PROCESS_OWNED_VMS.items() if task_name is None or rec == task_name
    ]
    if not matching_ids:
        return []
    owns_controller = controller is None
    if controller is None:
        try:
            controller = controller_factory()
        except Exception:
            logger.debug("Controller creation failed during process VM sweep", exc_info=True)
            return []
    try:
        return await _stop_vms(controller, matching_ids, reason="orphaned process-owned")
    finally:
        if owns_controller:
            with contextlib.suppress(Exception):
                await controller.close()


async def _teardown_failed_init(
    controller: CapsemController, vm_id: str, timeout: float = _INIT_TEARDOWN_TIMEOUT_SECS
) -> None:
    try:
        await asyncio.wait_for(asyncio.shield(controller.stop_vm(vm_id)), timeout=timeout)
        _unregister_process_owned_vm(vm_id)
    except Exception as exc:
        logger.warning(
            "Cleanup after failed sample_init of VM %s failed "
            "(run 'inspect sandbox cleanup capsem' to retry): %s",
            vm_id,
            exc,
        )


async def sweep_leftover_vms(controller: CapsemController) -> list[str]:
    """Delete leaked `inspect-capsem` VMs in a terminal state; return their ids."""
    try:
        vms = await controller.list_vms()
    except Exception:
        logger.debug("list_vms failed during leftover VM sweep", exc_info=True)
        return []
    prefix, owned = _managed_vm_prefix_slug(), set(_PROCESS_OWNED_VMS.keys())
    target_ids = [
        m.id
        for m in vms
        if m.id
        and _is_managed_vm(m, prefix)
        and m.id not in owned
        and m.status in _TERMINAL_VM_STATES
    ]
    return (
        await _stop_vms(controller, target_ids, reason="leftover inspect-capsem")
        if target_ids
        else []
    )
