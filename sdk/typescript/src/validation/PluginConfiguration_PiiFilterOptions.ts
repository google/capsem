// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_PiiFilterOptions} from "../models/PluginConfiguration_PiiFilterOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_PiiFilterOptionsModeSchema} from "./PluginConfiguration_PiiFilterOptionsMode.js";
import {PluginConfiguration_PiiFilterOptionsOptionsSchema} from "./PluginConfiguration_PiiFilterOptionsOptions.js";

export const PluginConfiguration_PiiFilterOptionsSchema: z.ZodType<PluginConfiguration_PiiFilterOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.lazy(() => PluginConfiguration_PiiFilterOptionsModeSchema).exactOptional(),
  "options": z.lazy(() => PluginConfiguration_PiiFilterOptionsOptionsSchema).exactOptional(),
});
