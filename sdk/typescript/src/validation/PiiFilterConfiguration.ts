// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PiiFilterConfiguration} from "../models/PiiFilterConfiguration.js";
import {PluginConfiguration_PiiFilterOptionsSchema} from "./PluginConfiguration_PiiFilterOptions.js";

export const PiiFilterConfigurationSchema: z.ZodType<PiiFilterConfiguration> = z.strictObject({
  "inbound": z.lazy(() => PluginConfiguration_PiiFilterOptionsSchema).exactOptional(),
  "outbound": z.lazy(() => PluginConfiguration_PiiFilterOptionsSchema).exactOptional(),
});
