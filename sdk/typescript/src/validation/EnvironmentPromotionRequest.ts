// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPromotionRequest} from "../models/EnvironmentPromotionRequest.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentPromotionRequestSchema: z.ZodType<EnvironmentPromotionRequest> = z.strictObject({
  "base_revision_id": z.uuid(),
  "reason": z.string(),
  "request_id": z.uuid(),
  "source": z.lazy(() => EnvironmentRevisionReferenceSchema),
});
