"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .model_base import Model
from .pii_filter_configuration import PiiFilterConfiguration
from .plugin_configuration__credential_broker_options import (
    PluginConfiguration_CredentialBrokerOptions,
)
from .plugin_configuration__dummy_post_allow_options import (
    PluginConfiguration_DummyPostAllowOptions,
)
from .plugin_configuration__dummy_pre_eicar_options import (
    PluginConfiguration_DummyPreEicarOptions,
)
from .plugin_configuration__log_sanitizer_options import (
    PluginConfiguration_LogSanitizerOptions,
)
from .plugin_configuration__secret_exfiltration_options import (
    PluginConfiguration_SecretExfiltrationOptions,
)
from .plugin_configuration__utf8_guard_options import (
    PluginConfiguration_Utf8GuardOptions,
)


class EnvironmentPlugins(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    credential_broker: PluginConfiguration_CredentialBrokerOptions | None = None
    dummy_post_allow: PluginConfiguration_DummyPostAllowOptions | None = None
    dummy_pre_eicar: PluginConfiguration_DummyPreEicarOptions | None = None
    log_sanitizer: PluginConfiguration_LogSanitizerOptions | None = None
    pii_filter: PiiFilterConfiguration | None = None
    secret_exfiltration: PluginConfiguration_SecretExfiltrationOptions | None = None
    utf8_guard: PluginConfiguration_Utf8GuardOptions | None = None
