// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CapabilitySummary} from "../models/CapabilitySummary.js";

export const CapabilitySummarySchema: z.ZodType<CapabilitySummary> = z.strictObject({
  "consumer": z.string(),
  "id": z.string(),
  "label": z.string(),
  "required_scopes": z.array(z.string()),
  "supported": z.boolean(),
});
