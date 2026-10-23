// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {BrowserAcknowledgeResponse} from "../models/BrowserAcknowledgeResponse.js";

export const BrowserAcknowledgeResponseSchema: z.ZodType<BrowserAcknowledgeResponse> = z.object({
  "owner_generation": z.string(),
  "request_id": z.int().min(0),
  "vm_id": z.string(),
});
