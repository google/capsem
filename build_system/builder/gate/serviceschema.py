"""Service and smoke-test configuration owned by the development gate."""

from __future__ import annotations

from .configschema import Strict


class ServiceConfig(Strict):
    """The development daemon, on the same rail an installed package uses."""

    binary: str
    process_binary: str
    sync_assets_script: str
    generated_profiles: str
    assets_dir: str
    home_assets: str
    home_profiles: str
    socket: str
    pidfile: str
    retired_config: tuple[str, ...]
    ready_attempts: int
    ready_interval_seconds: float
    log_level: str


class SmokeGroup(Strict):
    name: str
    paths: tuple[str, ...]
    markers: str
    parallel: int = 0


class SmokeConfig(Strict):
    doctor: tuple[str, ...]
    run_id_variable: str
    log: str
    groups: tuple[SmokeGroup, ...]
    serial_groups: tuple[SmokeGroup, ...]
