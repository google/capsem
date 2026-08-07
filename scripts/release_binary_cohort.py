"""Canonical native Linux binary cohort required by release qualification."""

from __future__ import annotations

REQUIRED_LINUX_RELEASE_BINARIES = frozenset(
    {
        "capsem",
        "capsem-admin",
        "capsem-app",
        "capsem-bench-rs",
        "capsem-gateway",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-mock-server",
        "capsem-process",
        "capsem-service",
        "capsem-tray",
        "capsem-tui",
    }
)
