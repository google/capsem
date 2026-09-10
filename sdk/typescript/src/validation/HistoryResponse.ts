// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HistoryResponse} from "../models/HistoryResponse.js";
import {HistoryEntrySchema} from "./HistoryEntry.js";

export const HistoryResponseSchema: z.ZodType<HistoryResponse> = z.object({
  "commands": z.array(z.lazy(() => HistoryEntrySchema)),
  "has_more": z.boolean(),
  "total": z.int().min(0),
});
