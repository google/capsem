"""VM-scoped session export (`vm.export.quiesce`, `rows`, `warc`, `events`, and `atif`)."""

from __future__ import annotations

import asyncio
import json
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any, overload

from . import _operations as api
from ._resources import Resource
from ._trace_bodies import BodiesMode, WarcBodyReader
from ._trace_ledger import (
    ALL_EXPORT_TABLES,
    DROPPED_SECURITY_TABLES,
    EXPORT_PERSIST_PREFIX,
    HARBOR_EVENT_TABLES,
    atomic_write_bytes,
    cleanup_export_persisted_vm,
    is_export_persisted_vm,
    load_rows_tables,
    quiesce_vm_for_export,
    resolve_include_security_events,
    write_export_meta,
)
from .trace import enrich_atif_trajectory, rows_to_atif

_ATIF_CORE_TABLES = (
    "model_calls",
    "model_items",
    "tool_calls",
    "tool_responses",
    "event_body_blobs",
    *HARBOR_EVENT_TABLES,
)
_ALL_TABLES_QUERY = ",".join(ALL_EXPORT_TABLES)


def _normalize_tables_param(tables: str | Sequence[str] | None) -> str | None:
    if tables is None or isinstance(tables, str):
        return tables
    return ",".join(tables)


class Export(Resource):
    """Export session artifacts (`quiesce`, `rows`, `warc`, `events`, `atif`) from a `VM`."""

    async def quiesce(self) -> bool:
        """Persist (if ephemeral) and stop this VM to drain its session ledger before export.

        Transitions a running VM to the `stopped` state so `FsMonitor`, the proxy
        worker, and `ledger-worker` flush and seal `session.bodies`.
        """
        resolve_fn = getattr(self._vm, "_resolve", None)
        vm_id = await resolve_fn() if callable(resolve_fn) else str(getattr(self._vm, "id", ""))
        return await quiesce_vm_for_export(self._vm, vm_id)

    async def cleanup_persisted(self) -> bool:
        """Delete this VM if it was temporarily persisted under `export-*` by `quiesce()`."""
        return await cleanup_export_persisted_vm(self._vm)

    @overload
    async def rows(
        self, output_path: None = None, *, tables: str | Sequence[str] | None = None
    ) -> bytes: ...

    @overload
    async def rows(
        self, output_path: str | Path, *, tables: str | Sequence[str] | None = None
    ) -> Path: ...

    async def rows(
        self,
        output_path: str | Path | None = None,
        *,
        tables: str | Sequence[str] | None = None,
    ) -> bytes | Path:
        """Download `GET /vms/{id}/export/rows.ndjson.gz` as gzip-compressed NDJSON bytes."""
        vm_id = await self._vm._resolve()
        data = await api.export_vm_rows(
            self._vm._transport,
            id=vm_id,
            tables=_normalize_tables_param(tables),
        )
        if output_path is None:
            return data
        out = Path(output_path)
        await asyncio.to_thread(atomic_write_bytes, out, data)
        return out

    @overload
    async def warc(self, output_path: None = None) -> bytes: ...

    @overload
    async def warc(self, output_path: str | Path) -> Path: ...

    async def warc(self, output_path: str | Path | None = None) -> bytes | Path:
        """Download `GET /vms/{id}/bodies/export.warc.gz` as gzip-compressed WARC 1.1 bytes."""
        vm_id = await self._vm._resolve()
        data = await api.export_vm_bodies(self._vm._transport, id=vm_id)
        if output_path is None:
            return data
        out = Path(output_path)
        await asyncio.to_thread(atomic_write_bytes, out, data)
        return out

    async def events(
        self,
        out_dir: str | Path,
        *,
        include_security_events: bool | None = None,
        labels: Mapping[str, str] | None = None,
        write_meta: bool = False,
        quiesce: bool = False,
        cleanup_persisted: bool = False,
    ) -> tuple[Path, Path]:
        """Download `<vm_id>.rows.ndjson.gz` and `<vm_id>.warc.gz` into `out_dir`."""
        vm_id = await self._vm._resolve()
        if quiesce:
            await quiesce_vm_for_export(self._vm, vm_id)
        keep_sec = resolve_include_security_events(include_security_events=include_security_events)
        tables_q = _ALL_TABLES_QUERY if keep_sec else None
        rows_bytes = await api.export_vm_rows(self._vm._transport, id=vm_id, tables=tables_q)
        warc_bytes = await api.export_vm_bodies(self._vm._transport, id=vm_id)
        if cleanup_persisted:
            await cleanup_export_persisted_vm(self._vm)
        dst_dir = Path(out_dir)
        rows_path = dst_dir / f"{vm_id}.rows.ndjson.gz"
        warc_path = dst_dir / f"{vm_id}.warc.gz"

        def _write() -> None:
            atomic_write_bytes(rows_path, rows_bytes)
            atomic_write_bytes(warc_path, warc_bytes)
            if write_meta or labels:
                write_export_meta(
                    dst_dir,
                    vm_id,
                    rows_path=rows_path,
                    warc_path=warc_path,
                    include_security_events=keep_sec,
                    labels=labels,
                )

        await asyncio.to_thread(_write)
        return rows_path, warc_path

    async def atif(
        self,
        trajectory: str | Path | dict[str, Any] | None = None,
        *,
        output_path: str | Path | None = None,
        bodies: BodiesMode = "http",
        include_security_events: bool | None = None,
        session_id: str | None = None,
        agent_name: str | None = None,
        agent_version: str | None = None,
        quiesce: bool = False,
        cleanup_persisted: bool = False,
    ) -> dict[str, Any]:
        """Synthesize or enrich an `ATIF-v1.7` trajectory for this VM over HTTP."""
        vm_id = await self._vm._resolve()
        if quiesce:
            await quiesce_vm_for_export(self._vm, vm_id)
        keep_sec = resolve_include_security_events(include_security_events=include_security_events)
        tables_q = _ALL_TABLES_QUERY if keep_sec else None
        t_names = (*_ATIF_CORE_TABLES, *(DROPPED_SECURITY_TABLES if keep_sec else ()))
        rows_bytes = await api.export_vm_rows(self._vm._transport, id=vm_id, tables=tables_q)
        warc_bytes = (
            await api.export_vm_bodies(self._vm._transport, id=vm_id)
            if bodies in ("warc", "http", "full")
            else None
        )
        if cleanup_persisted:
            await cleanup_export_persisted_vm(self._vm)

        out_p = Path(output_path) if output_path is not None else None

        def _build() -> dict[str, Any]:
            tables = load_rows_tables(rows_bytes, t_names)
            reader = WarcBodyReader(warc_bytes) if warc_bytes is not None else None
            if trajectory is None:
                return rows_to_atif(
                    tables,
                    vm_id=vm_id,
                    session_id=session_id,
                    agent_name=agent_name,
                    agent_version=agent_version,
                    include_security_events=keep_sec,
                    bodies=bodies,
                    body_reader=reader,
                    output_path=out_p,
                )
            if isinstance(trajectory, (str, Path)):
                traj_p = Path(trajectory)
                traj_data: dict[str, Any] = json.loads(traj_p.read_text(encoding="utf-8"))
                eff_out = out_p or traj_p
            else:
                traj_data = dict(trajectory)
                eff_out = out_p
            return enrich_atif_trajectory(
                traj_data,
                tables,
                vm_id=vm_id,
                include_security_events=keep_sec,
                bodies=bodies,
                body_reader=reader,
                output_path=eff_out,
            )

        return await asyncio.to_thread(_build)


__all__ = [
    "EXPORT_PERSIST_PREFIX",
    "Export",
    "cleanup_export_persisted_vm",
    "is_export_persisted_vm",
    "quiesce_vm_for_export",
]
