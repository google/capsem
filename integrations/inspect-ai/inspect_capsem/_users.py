"""Shared user-specification classification helpers for `inspect-capsem`."""

from __future__ import annotations


def is_root_user_spec(user: str | None) -> bool:
    """Return True when `user` is unset, empty, or resolves to root (`root`, `0`, `0:0`)."""
    if user is None:
        return True
    u, _, g = user.strip().partition(":")
    return u.strip().lower() in ("", "root", "0") and g.strip().lower() in ("", "root", "0")


__all__ = ["is_root_user_spec"]
