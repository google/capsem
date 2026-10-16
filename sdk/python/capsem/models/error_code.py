"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ErrorCode(StrEnum):
    VM_NOT_FOUND = 'vm_not_found'
    CREATE_TIMEOUT = 'create_timeout'
    EXEC_TIMEOUT = 'exec_timeout'
    CONFIGURATION_REQUIRED = 'configuration_required'
    UNSUPPORTED_CAPABILITY = 'unsupported_capability'
    UNKNOWN_SCOPE_PROFILE = 'unknown_scope_profile'
    POLICY_DENIED = 'policy_denied'
    SCOPE_DENIED = 'scope_denied'
    CONNECTION_UNAVAILABLE = 'connection_unavailable'
    REAUTH_REQUIRED = 'reauth_required'
    STORAGE_UNAVAILABLE = 'storage_unavailable'
    STORAGE_COMMIT_FAILED = 'storage_commit_failed'
    TRANSACTION_EXPIRED = 'transaction_expired'
    TRANSACTION_CANCELLED = 'transaction_cancelled'
    INVALID_CALLBACK = 'invalid_callback'
    PROVIDER_REJECTED = 'provider_rejected'
    PROVIDER_TIMEOUT = 'provider_timeout'
    OWNER_CHANGED = 'owner_changed'
    SESSION_UNAVAILABLE = 'session_unavailable'
    GRANT_UNAVAILABLE = 'grant_unavailable'
    REQUEST_CONFLICT = 'request_conflict'
    REVOCATION_PENDING = 'revocation_pending'
