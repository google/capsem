// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentApplyTarget} from "../models/EnvironmentApplyTarget.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentApplyTargetSchema: z.ZodType<EnvironmentApplyTarget> = z.strictObject({
  "base": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "generation": z.uuid(),
  "session_id": z.uuid(),
});
