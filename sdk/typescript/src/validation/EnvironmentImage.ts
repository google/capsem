// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentImage} from "../models/EnvironmentImage.js";
import {EnvironmentImageUpdatePolicySchema} from "./EnvironmentImageUpdatePolicy.js";

export const EnvironmentImageSchema: z.ZodType<EnvironmentImage> = z.strictObject({
  "reference": z.string(),
  "resolved_digest": z.string(),
  "update_policy": z.lazy(() => EnvironmentImageUpdatePolicySchema).exactOptional(),
});
