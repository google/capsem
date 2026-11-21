// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentTomlValidationRequest} from "../models/EnvironmentTomlValidationRequest.js";

export const EnvironmentTomlValidationRequestSchema: z.ZodType<EnvironmentTomlValidationRequest> = z.strictObject({
  "toml": z.string(),
});
