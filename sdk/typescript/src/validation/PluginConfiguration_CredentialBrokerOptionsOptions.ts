// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_CredentialBrokerOptionsOptions} from "../models/PluginConfiguration_CredentialBrokerOptionsOptions.js";

export const PluginConfiguration_CredentialBrokerOptionsOptionsSchema: z.ZodType<PluginConfiguration_CredentialBrokerOptionsOptions> = z.strictObject({
  "capture_enabled": z.boolean().exactOptional(),
});
