// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HostLogsResponse} from "../models/HostLogsResponse.js";
import {HostLogSourceSchema} from "./HostLogSource.js";

export const HostLogsResponseSchema: z.ZodType<HostLogsResponse> = z.object({
  "source": z.lazy(() => HostLogSourceSchema),
  "text": z.string(),
});
