// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPlugins} from "../models/EnvironmentPlugins.js";
import {PiiFilterConfigurationSchema} from "./PiiFilterConfiguration.js";
import {PluginConfiguration_CredentialBrokerOptionsSchema} from "./PluginConfiguration_CredentialBrokerOptions.js";
import {PluginConfiguration_DummyPostAllowOptionsSchema} from "./PluginConfiguration_DummyPostAllowOptions.js";
import {PluginConfiguration_DummyPreEicarOptionsSchema} from "./PluginConfiguration_DummyPreEicarOptions.js";
import {PluginConfiguration_LogSanitizerOptionsSchema} from "./PluginConfiguration_LogSanitizerOptions.js";
import {PluginConfiguration_SecretExfiltrationOptionsSchema} from "./PluginConfiguration_SecretExfiltrationOptions.js";
import {PluginConfiguration_Utf8GuardOptionsSchema} from "./PluginConfiguration_Utf8GuardOptions.js";

export const EnvironmentPluginsSchema: z.ZodType<EnvironmentPlugins> = z.strictObject({
  "credential_broker": z.union([z.null(), z.lazy(() => PluginConfiguration_CredentialBrokerOptionsSchema)]).exactOptional(),
  "dummy_post_allow": z.union([z.null(), z.lazy(() => PluginConfiguration_DummyPostAllowOptionsSchema)]).exactOptional(),
  "dummy_pre_eicar": z.union([z.null(), z.lazy(() => PluginConfiguration_DummyPreEicarOptionsSchema)]).exactOptional(),
  "log_sanitizer": z.union([z.null(), z.lazy(() => PluginConfiguration_LogSanitizerOptionsSchema)]).exactOptional(),
  "pii_filter": z.union([z.null(), z.lazy(() => PiiFilterConfigurationSchema)]).exactOptional(),
  "secret_exfiltration": z.union([z.null(), z.lazy(() => PluginConfiguration_SecretExfiltrationOptionsSchema)]).exactOptional(),
  "utf8_guard": z.union([z.null(), z.lazy(() => PluginConfiguration_Utf8GuardOptionsSchema)]).exactOptional(),
});
