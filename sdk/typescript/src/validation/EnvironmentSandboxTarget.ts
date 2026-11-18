// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSandboxTarget} from "../models/EnvironmentSandboxTarget.js";

export const EnvironmentSandboxTargetSchema: z.ZodType<EnvironmentSandboxTarget> = z.strictObject({
  "generation": z.uuid(),
  "session_id": z.uuid(),
});
