// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentApplyPreviewResponse} from "../models/EnvironmentApplyPreviewResponse.js";
import {EnvironmentApplyTargetPreviewSchema} from "./EnvironmentApplyTargetPreview.js";

export const EnvironmentApplyPreviewResponseSchema: z.ZodType<EnvironmentApplyPreviewResponse> = z.object({
  "targets": z.array(z.lazy(() => EnvironmentApplyTargetPreviewSchema)),
});
