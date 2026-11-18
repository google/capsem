// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_LogSanitizerOptions} from "../models/PluginConfiguration_LogSanitizerOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_LogSanitizerOptionsOptionsSchema} from "./PluginConfiguration_LogSanitizerOptionsOptions.js";

export const PluginConfiguration_LogSanitizerOptionsSchema: z.ZodType<PluginConfiguration_LogSanitizerOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.literal("rewrite").exactOptional(),
  "options": z.lazy(() => PluginConfiguration_LogSanitizerOptionsOptionsSchema).exactOptional(),
});
