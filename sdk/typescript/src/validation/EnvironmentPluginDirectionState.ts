// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPluginDirectionState} from "../models/EnvironmentPluginDirectionState.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {PluginDirectionSchema} from "./PluginDirection.js";
import {SecurityPluginModeSchema} from "./SecurityPluginMode.js";

export const EnvironmentPluginDirectionStateSchema: z.ZodType<EnvironmentPluginDirectionState> = z.strictObject({
  "detection_level": z.union([z.null(), z.lazy(() => DetectionLevelSchema)]),
  "direction": z.lazy(() => PluginDirectionSchema),
  "mode": z.lazy(() => SecurityPluginModeSchema),
});
