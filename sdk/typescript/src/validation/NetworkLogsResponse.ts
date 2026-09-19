// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {NetworkLogsResponse} from "../models/NetworkLogsResponse.js";
import {NetworkLogEventSchema} from "./NetworkLogEvent.js";

export const NetworkLogsResponseSchema: z.ZodType<NetworkLogsResponse> = z.object({
  "cursor": z.string(),
  "events": z.array(z.lazy(() => NetworkLogEventSchema)),
  "next_cursor": z.string().nullable().exactOptional(),
});
