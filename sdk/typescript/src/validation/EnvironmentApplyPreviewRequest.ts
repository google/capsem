// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentApplyPreviewRequest} from "../models/EnvironmentApplyPreviewRequest.js";
import {EnvironmentApplyTargetSchema} from "./EnvironmentApplyTarget.js";

export const EnvironmentApplyPreviewRequestSchema: z.ZodType<EnvironmentApplyPreviewRequest> = z.strictObject({
  "base_revision_id": z.uuid(),
  "revision_id": z.uuid(),
  "targets": z.array(z.lazy(() => EnvironmentApplyTargetSchema)),
});
