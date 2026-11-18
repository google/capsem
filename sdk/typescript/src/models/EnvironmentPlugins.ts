// Generated from Capsem OpenAPI. Do not edit.

import type { PiiFilterConfiguration } from "./PiiFilterConfiguration.js";
import type { PluginConfiguration_CredentialBrokerOptions } from "./PluginConfiguration_CredentialBrokerOptions.js";
import type { PluginConfiguration_DummyPostAllowOptions } from "./PluginConfiguration_DummyPostAllowOptions.js";
import type { PluginConfiguration_DummyPreEicarOptions } from "./PluginConfiguration_DummyPreEicarOptions.js";
import type { PluginConfiguration_LogSanitizerOptions } from "./PluginConfiguration_LogSanitizerOptions.js";
import type { PluginConfiguration_SecretExfiltrationOptions } from "./PluginConfiguration_SecretExfiltrationOptions.js";
import type { PluginConfiguration_Utf8GuardOptions } from "./PluginConfiguration_Utf8GuardOptions.js";

export interface EnvironmentPlugins {
  "credential_broker"?: null | PluginConfiguration_CredentialBrokerOptions;
  "dummy_post_allow"?: null | PluginConfiguration_DummyPostAllowOptions;
  "dummy_pre_eicar"?: null | PluginConfiguration_DummyPreEicarOptions;
  "log_sanitizer"?: null | PluginConfiguration_LogSanitizerOptions;
  "pii_filter"?: null | PiiFilterConfiguration;
  "secret_exfiltration"?: null | PluginConfiguration_SecretExfiltrationOptions;
  "utf8_guard"?: null | PluginConfiguration_Utf8GuardOptions;
}
