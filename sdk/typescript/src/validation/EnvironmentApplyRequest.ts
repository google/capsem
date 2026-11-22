// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentApplyRequest} from "../models/EnvironmentApplyRequest.js";
import {EnvironmentApplyTargetSchema} from "./EnvironmentApplyTarget.js";

export const EnvironmentApplyRequestSchema: z.ZodType<EnvironmentApplyRequest> = z.strictObject({
  "base_revision_id": z.uuid(),
  "reason": z.string(),
  "request_id": z.uuid(),
  "revision_id": z.uuid(),
  "targets": z.array(z.lazy(() => EnvironmentApplyTargetSchema)),
});
