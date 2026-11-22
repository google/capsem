// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentImageUpdateRequest} from "../models/EnvironmentImageUpdateRequest.js";
import {EnvironmentImageUpdateActionSchema} from "./EnvironmentImageUpdateAction.js";

export const EnvironmentImageUpdateRequestSchema: z.ZodType<EnvironmentImageUpdateRequest> = z.strictObject({
  "action": z.lazy(() => EnvironmentImageUpdateActionSchema),
  "base_revision_id": z.uuid(),
  "expected_digest": z.string(),
  "reason": z.string(),
  "request_id": z.uuid(),
});
