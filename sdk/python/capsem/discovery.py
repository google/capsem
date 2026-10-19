"""Resolve one complete connection without combining credential sources."""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path

from ._local import capsem_run_dir as capsem_run_dir
from ._local import local_connection

DEFAULT_GATEWAY_PORT = 19222
DEFAULT_GATEWAY_URL = f"http://127.0.0.1:{DEFAULT_GATEWAY_PORT}"


@dataclass(frozen=True, slots=True)
class GatewayEndpoint:
    """Resolved HTTP gateway base URL and bearer token."""

    url: str
    token: str = field(repr=False)
    run_dir: Path | None = None


def discover_gateway(
    *, url: str | None = None, token: str | None = None, run_dir: str | Path | None = None,
) -> GatewayEndpoint:
    """Use a complete explicit pair, a complete environment pair, or private local metadata."""
    if url is not None or token is not None:
        if url is None or token is None:
            raise ValueError("provide both gateway URL and token, or neither for local Capsem")
        return GatewayEndpoint(url=url, token=token)
    env_url = os.environ.get("CAPSEM_GATEWAY_URL")
    env_token = os.environ.get("CAPSEM_GATEWAY_TOKEN")
    if env_url is not None or env_token is not None:
        if not env_url or not env_token:
            raise ValueError("provide both CAPSEM_GATEWAY_URL and CAPSEM_GATEWAY_TOKEN")
        return GatewayEndpoint(url=env_url.strip().rstrip("/"), token=env_token)
    directory = capsem_run_dir(run_dir)
    local_url, local_token = local_connection(directory)
    return GatewayEndpoint(url=local_url, token=local_token, run_dir=directory)
