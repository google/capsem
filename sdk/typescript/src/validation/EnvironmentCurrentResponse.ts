// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCurrentResponse} from "../models/EnvironmentCurrentResponse.js";
import {EnvironmentDocumentSchema} from "./EnvironmentDocument.js";

export const EnvironmentCurrentResponseSchema: z.ZodType<EnvironmentCurrentResponse> = z.strictObject({
  "actor": z.string(),
  "archived": z.boolean(),
  "document": z.lazy(() => EnvironmentDocumentSchema),
  "reason": z.string(),
  "request_id": z.string().nullable().exactOptional(),
  "timestamp_unix_ms": z.int(),
});
