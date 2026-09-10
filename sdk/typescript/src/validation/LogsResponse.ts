// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {LogsResponse} from "../models/LogsResponse.js";

export const LogsResponseSchema: z.ZodType<LogsResponse> = z.object({
  "logs": z.string(),
  "process_logs": z.string().nullable().exactOptional(),
  "serial_logs": z.string().nullable().exactOptional(),
});
