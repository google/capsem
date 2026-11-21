"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class PluginId(StrEnum):
    PII_FILTER = 'pii_filter'
    SECRET_EXFILTRATION = 'secret_exfiltration'
    UTF8_GUARD = 'utf8_guard'
    CREDENTIAL_BROKER = 'credential_broker'
    LOG_SANITIZER = 'log_sanitizer'
    DUMMY_PRE_EICAR = 'dummy_pre_eicar'
    DUMMY_POST_ALLOW = 'dummy_post_allow'
