// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_Utf8GuardOptions} from "../models/PluginConfiguration_Utf8GuardOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_Utf8GuardOptionsModeSchema} from "./PluginConfiguration_Utf8GuardOptionsMode.js";
import {PluginConfiguration_Utf8GuardOptionsOptionsSchema} from "./PluginConfiguration_Utf8GuardOptionsOptions.js";

export const PluginConfiguration_Utf8GuardOptionsSchema: z.ZodType<PluginConfiguration_Utf8GuardOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.lazy(() => PluginConfiguration_Utf8GuardOptionsModeSchema).exactOptional(),
  "options": z.lazy(() => PluginConfiguration_Utf8GuardOptionsOptionsSchema).exactOptional(),
});
