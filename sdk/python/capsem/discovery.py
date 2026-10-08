"""Host gateway URL and bearer token discovery from environment and `~/.capsem/run`."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

DEFAULT_GATEWAY_PORT = 19222
DEFAULT_GATEWAY_URL = f"http://127.0.0.1:{DEFAULT_GATEWAY_PORT}"


@dataclass(frozen=True, slots=True)
class GatewayEndpoint:
    """Resolved HTTP gateway base URL and bearer token."""

    url: str
    token: str
    run_dir: Path | None = None


def capsem_run_dir(run_dir: str | Path | None = None) -> Path:
    """Resolve the Capsem runtime directory matching `capsem_foundation::paths::capsem_run_dir`.

    Priority:
    1. Explicit `run_dir` argument (when provided and non-empty).
    2. `CAPSEM_RUN_DIR` environment variable.
    3. `CAPSEM_HOME/run` when `CAPSEM_HOME` is set.
    4. `~/.capsem/run`.
    """
    if run_dir is not None:
        raw = str(run_dir).strip()
        if not raw:
            raise ValueError("run_dir must be a nonempty path")
        return Path(raw).expanduser()
    env_run = os.environ.get("CAPSEM_RUN_DIR", "").strip()
    if env_run:
        return Path(env_run).expanduser()
    env_home = os.environ.get("CAPSEM_HOME", "").strip()
    if env_home:
        return Path(env_home).expanduser() / "run"
    return Path.home() / ".capsem" / "run"


def _resolve_gateway_url(url: str | None, resolved_run_dir: Path) -> str:
    if url is not None:
        return url
    env_url = os.environ.get("CAPSEM_GATEWAY_URL", "").strip()
    if env_url:
        return env_url.rstrip("/")
    port_file = resolved_run_dir / "gateway.port"
    if port_file.is_file():
        raw_port = port_file.read_text(encoding="utf-8").strip()
        if raw_port:
            try:
                port = int(raw_port)
            except ValueError as error:
                raise ValueError(f"invalid port in {port_file}") from error
            if not 1 <= port <= 65535:
                raise ValueError(f"invalid port in {port_file}")
            return f"http://127.0.0.1:{port}"
    return DEFAULT_GATEWAY_URL


def _resolve_gateway_token(token: str | None, resolved_run_dir: Path) -> str:
    if token is not None:
        return token
    env_token = os.environ.get("CAPSEM_GATEWAY_TOKEN", "").strip()
    if env_token:
        return env_token
    token_file = resolved_run_dir / "gateway.token"
    if token_file.is_file():
        file_token = token_file.read_text(encoding="utf-8").strip()
        if file_token:
            return file_token
    raise RuntimeError(f"gateway bearer token not found in CAPSEM_GATEWAY_TOKEN or {token_file}")


def discover_gateway(
    *, url: str | None = None, token: str | None = None, run_dir: str | Path | None = None,
) -> GatewayEndpoint:
    """Resolve the local gateway URL and bearer token from arguments, environment, or `run_dir`."""
    resolved_run_dir = capsem_run_dir(run_dir)
    return GatewayEndpoint(
        url=_resolve_gateway_url(url, resolved_run_dir),
        token=_resolve_gateway_token(token, resolved_run_dir),
        run_dir=resolved_run_dir,
    )
