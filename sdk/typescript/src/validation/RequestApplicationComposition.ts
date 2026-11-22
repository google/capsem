// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestApplicationComposition} from "../models/RequestApplicationComposition.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const RequestApplicationCompositionSchema: z.ZodType<RequestApplicationComposition> = z.strictObject({
  "boot": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "corporate_policy_id": z.string(),
  "desired": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "live_policy": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "pending_restart": z.boolean(),
});
