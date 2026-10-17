"""Surviving VM reporting and untracked managed VM sweep helpers for CLI cleanup."""

from __future__ import annotations

import contextlib
import logging
from collections.abc import Callable
from typing import TYPE_CHECKING, Any

from inspect_capsem._controller import (
    _MANAGED_BY_LABEL,
    _MANAGED_BY_VALUE,
    _is_managed_vm,
    _managed_vm_prefix_slug,
)

if TYPE_CHECKING:
    from inspect_capsem._controller import CapsemController

logger = logging.getLogger(__name__)


def _report_surviving_vms(surviving_ids: list[str]) -> None:
    if not surviving_ids:
        return
    if len(surviving_ids) == 1:
        sid = surviving_ids[0]
        print(
            f"Capsem sandbox VM left running: {sid}\n"
            f"Clean up with: inspect sandbox cleanup capsem {sid}"
        )
    else:
        cleanup = "\n".join(f"  inspect sandbox cleanup capsem {sid}" for sid in surviving_ids)
        joined = ", ".join(surviving_ids)
        print(f"Capsem sandbox VMs left running: {joined}\nClean up with:\n{cleanup}")


async def _abandon_task_environments(
    active_environments: dict[str, Any],
    envs: list[Any],
    surviving_ids: list[str],
) -> None:
    for env in envs:
        active_environments.pop(env._instance_id, None)
        if env._vm_id and env._vm_id not in surviving_ids:
            surviving_ids.append(env._vm_id)
        if env._owns_controller:
            env._owns_controller = False
            with contextlib.suppress(Exception):
                await env._controller.close()
    _report_surviving_vms(surviving_ids)


async def _cleanup_environment(env: Any, unregister_vm: Callable[[str], None]) -> None:
    env._active_environments.pop(env._instance_id, None)
    try:
        if not env._cleaned_up and env._vm_id:
            try:
                await env._controller.stop_vm(env._vm_id)
                env._cleaned_up = True
                unregister_vm(env._vm_id)
            except Exception as exc:
                logger.warning("Failed to stop Capsem VM %s: %s", env._vm_id, exc)
    finally:
        if env._owns_controller:
            env._owns_controller = False
            with contextlib.suppress(Exception):
                await env._controller.close()


async def _sweep_untracked_managed_vms(
    controller_factory: Callable[[], CapsemController],
    id_filter: str | None,
    stopped_vms: set[str],
    unregister_vm: Callable[[str], None] | None = None,
) -> None:
    prefix_slug = _managed_vm_prefix_slug()
    controller: CapsemController | None = None
    try:
        try:
            controller = controller_factory()
            vms = await controller.list_vms()
        except Exception:
            logger.warning("Controller list_vms failed during cli_cleanup", exc_info=True)
            return
        for vm_info in vms:
            vid = vm_info.id or vm_info.name or ""
            vname = vm_info.name or vid
            if not vid or vid in stopped_vms:
                continue
            labels = vm_info.labels or {}
            has_managed = labels.get(_MANAGED_BY_LABEL) == _MANAGED_BY_VALUE
            is_managed = _is_managed_vm(vm_info, prefix_slug)
            if id_filter is not None:
                if id_filter not in (vid, vname):
                    continue
                if not is_managed:
                    reason = (
                        "was not created by inspect-capsem"
                        if not has_managed
                        else "is a named or persistent VM"
                        if vm_info.persistent
                        else "belongs to a different inspect-capsem prefix"
                    )
                    logger.warning(
                        "VM %r %s; use 'capsem' directly to manage it", id_filter, reason
                    )
                    continue
            elif not is_managed:
                continue
            try:
                await controller.stop_vm(vid)
                if unregister_vm is not None:
                    unregister_vm(vid)
                stopped_vms.add(vid)
            except Exception:
                logger.warning("Failed to clean up Capsem VM %s", vid, exc_info=True)
    finally:
        if controller is not None:
            with contextlib.suppress(Exception):
                await controller.close()
