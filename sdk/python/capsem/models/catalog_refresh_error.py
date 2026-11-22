"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class CatalogRefreshError(StrEnum):
    MISSING_SOURCE = 'missing_source'
    INVALID_SOURCE = 'invalid_source'
    POLICY_UNAVAILABLE = 'policy_unavailable'
    REGISTRY_UNAVAILABLE = 'registry_unavailable'
    VERIFICATION_FAILED = 'verification_failed'
    LOCAL_UNAVAILABLE = 'local_unavailable'
    UNSUPPORTED = 'unsupported'
