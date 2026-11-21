// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginControlGroup} from "../models/PluginControlGroup.js";

export const PluginControlGroupSchema: z.ZodType<PluginControlGroup> = z.object({
  "name": z.string(),
  "path": z.string(),
});
