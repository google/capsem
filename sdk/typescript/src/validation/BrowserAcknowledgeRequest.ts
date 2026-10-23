// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {BrowserAcknowledgeRequest} from "../models/BrowserAcknowledgeRequest.js";

export const BrowserAcknowledgeRequestSchema: z.ZodType<BrowserAcknowledgeRequest> = z.strictObject({
  "owner_generation": z.string(),
  "request_id": z.int().min(0),
});
