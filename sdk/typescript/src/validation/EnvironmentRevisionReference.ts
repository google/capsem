// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRevisionReference} from "../models/EnvironmentRevisionReference.js";

export const EnvironmentRevisionReferenceSchema: z.ZodType<EnvironmentRevisionReference> = z.strictObject({
  "environment_id": z.uuid(),
  "revision_id": z.uuid(),
});
