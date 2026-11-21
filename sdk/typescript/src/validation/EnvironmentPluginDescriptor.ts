// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPluginDescriptor} from "../models/EnvironmentPluginDescriptor.js";
import {EnvironmentPluginFieldSchema} from "./EnvironmentPluginField.js";
import {PluginControlGroupSchema} from "./PluginControlGroup.js";
import {PluginDetailCapabilitySchema} from "./PluginDetailCapability.js";
import {PluginDirectionSchema} from "./PluginDirection.js";
import {PluginEventFamilySchema} from "./PluginEventFamily.js";
import {PluginIdSchema} from "./PluginId.js";
import {PluginStageSchema} from "./PluginStage.js";
import {SecurityPluginModeSchema} from "./SecurityPluginMode.js";

export const EnvironmentPluginDescriptorSchema: z.ZodType<EnvironmentPluginDescriptor> = z.strictObject({
  "description": z.string(),
  "detail_capabilities": z.array(z.lazy(() => PluginDetailCapabilitySchema)),
  "diagnostic": z.boolean(),
  "directions": z.array(z.lazy(() => PluginDirectionSchema)),
  "event_families": z.array(z.lazy(() => PluginEventFamilySchema)),
  "fields": z.array(z.lazy(() => EnvironmentPluginFieldSchema)),
  "groups": z.array(z.lazy(() => PluginControlGroupSchema)),
  "id": z.lazy(() => PluginIdSchema),
  "limitations": z.array(z.string()),
  "modes": z.array(z.lazy(() => SecurityPluginModeSchema)),
  "name": z.string(),
  "required": z.boolean(),
  "stage": z.lazy(() => PluginStageSchema),
  "version": z.int().min(0),
});
