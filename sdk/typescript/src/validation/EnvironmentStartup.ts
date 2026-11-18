// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentStartup} from "../models/EnvironmentStartup.js";

export const EnvironmentStartupSchema: z.ZodType<EnvironmentStartup> = z.strictObject({
  "script": z.string(),
  "timeout_seconds": z.int().min(0),
});
