"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ErrorCode(StrEnum):
    VM_NOT_FOUND = 'vm_not_found'
    CREATE_TIMEOUT = 'create_timeout'
    EXEC_TIMEOUT = 'exec_timeout'
