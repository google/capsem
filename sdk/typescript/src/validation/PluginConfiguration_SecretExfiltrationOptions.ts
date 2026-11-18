// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_SecretExfiltrationOptions} from "../models/PluginConfiguration_SecretExfiltrationOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_SecretExfiltrationOptionsModeSchema} from "./PluginConfiguration_SecretExfiltrationOptionsMode.js";
import {PluginConfiguration_SecretExfiltrationOptionsOptionsSchema} from "./PluginConfiguration_SecretExfiltrationOptionsOptions.js";

export const PluginConfiguration_SecretExfiltrationOptionsSchema: z.ZodType<PluginConfiguration_SecretExfiltrationOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.lazy(() => PluginConfiguration_SecretExfiltrationOptionsModeSchema).exactOptional(),
  "options": z.lazy(() => PluginConfiguration_SecretExfiltrationOptionsOptionsSchema).exactOptional(),
});
