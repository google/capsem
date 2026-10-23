// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {BrowserRequest} from "../models/BrowserRequest.js";

export const BrowserRequestSchema: z.ZodType<BrowserRequest> = z.object({
  "id": z.int().min(0),
  "owner_generation": z.string().nullable().exactOptional(),
  "url": z.string().max(8192),
});
