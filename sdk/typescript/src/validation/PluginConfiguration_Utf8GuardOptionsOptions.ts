// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PluginConfiguration_Utf8GuardOptionsOptions} from "../models/PluginConfiguration_Utf8GuardOptionsOptions.js";

export const PluginConfiguration_Utf8GuardOptionsOptionsSchema: z.ZodType<PluginConfiguration_Utf8GuardOptionsOptions> = z.strictObject({
  "inbound": z.boolean().exactOptional(),
  "outbound": z.boolean().exactOptional(),
});
