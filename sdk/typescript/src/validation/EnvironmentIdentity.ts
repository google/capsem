// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentIdentity} from "../models/EnvironmentIdentity.js";

export const EnvironmentIdentitySchema: z.ZodType<EnvironmentIdentity> = z.strictObject({
  "id": z.string(),
  "parent_environment_id": z.string().nullable().exactOptional(),
  "parent_revision_id": z.string().nullable().exactOptional(),
  "revision_id": z.string(),
  "schema_version": z.int().min(0),
});
