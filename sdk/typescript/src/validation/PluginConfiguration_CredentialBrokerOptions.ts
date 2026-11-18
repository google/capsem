// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_CredentialBrokerOptions} from "../models/PluginConfiguration_CredentialBrokerOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_CredentialBrokerOptionsOptionsSchema} from "./PluginConfiguration_CredentialBrokerOptionsOptions.js";

export const PluginConfiguration_CredentialBrokerOptionsSchema: z.ZodType<PluginConfiguration_CredentialBrokerOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.literal("rewrite").exactOptional(),
  "options": z.lazy(() => PluginConfiguration_CredentialBrokerOptionsOptionsSchema).exactOptional(),
});
