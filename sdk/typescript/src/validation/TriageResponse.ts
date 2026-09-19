// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TriageResponse} from "../models/TriageResponse.js";
import {HostTriageResponseSchema} from "./HostTriageResponse.js";

export const TriageResponseSchema: z.ZodType<TriageResponse> = z.object({
  "host": z.lazy(() => HostTriageResponseSchema),
  "rank": z.array(z.string()),
  "session": z.json(),
  "session_id": z.string().nullable().exactOptional(),
  "since": z.string(),
});
