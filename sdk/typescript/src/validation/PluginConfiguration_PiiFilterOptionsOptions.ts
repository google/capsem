// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_PiiFilterOptionsOptions} from "../models/PluginConfiguration_PiiFilterOptionsOptions.js";

export const PluginConfiguration_PiiFilterOptionsOptionsSchema: z.ZodType<PluginConfiguration_PiiFilterOptionsOptions> = z.strictObject({
  "card": z.boolean().exactOptional(),
  "email": z.boolean().exactOptional(),
  "phone": z.boolean().exactOptional(),
  "ssn": z.boolean().exactOptional(),
});
