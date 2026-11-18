// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSecretBinding} from "../models/EnvironmentSecretBinding.js";

export const EnvironmentSecretBindingSchema: z.ZodType<EnvironmentSecretBinding> = z.strictObject({
  "binding_id": z.string(),
  "env_name": z.string().nullable().exactOptional(),
});
