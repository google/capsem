// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_DummyPreEicarOptions} from "../models/PluginConfiguration_DummyPreEicarOptions.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginConfiguration_DummyPreEicarOptionsModeSchema} from "./PluginConfiguration_DummyPreEicarOptionsMode.js";
import {PluginConfiguration_DummyPreEicarOptionsOptionsSchema} from "./PluginConfiguration_DummyPreEicarOptionsOptions.js";

export const PluginConfiguration_DummyPreEicarOptionsSchema: z.ZodType<PluginConfiguration_DummyPreEicarOptions> = z.strictObject({
  "detection_level": z.lazy(() => DetectionLevelSchema).exactOptional(),
  "mode": z.lazy(() => PluginConfiguration_DummyPreEicarOptionsModeSchema).exactOptional(),
  "options": z.lazy(() => PluginConfiguration_DummyPreEicarOptionsOptionsSchema).exactOptional(),
});
