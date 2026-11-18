// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_DummyPostAllowOptions} from "../models/PluginConfiguration_DummyPostAllowOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_DummyPostAllowOptionsModeSchema} from "./PluginConfiguration_DummyPostAllowOptionsMode.js";
import {PluginConfiguration_DummyPostAllowOptionsOptionsSchema} from "./PluginConfiguration_DummyPostAllowOptionsOptions.js";

export const PluginConfiguration_DummyPostAllowOptionsSchema: z.ZodType<PluginConfiguration_DummyPostAllowOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.lazy(() => PluginConfiguration_DummyPostAllowOptionsModeSchema).exactOptional(),
  "options": z.lazy(() => PluginConfiguration_DummyPostAllowOptionsOptionsSchema).exactOptional(),
});
