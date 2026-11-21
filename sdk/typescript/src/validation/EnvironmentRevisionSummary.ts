// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRevisionSummary} from "../models/EnvironmentRevisionSummary.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentRevisionSummarySchema: z.ZodType<EnvironmentRevisionSummary> = z.strictObject({
  "actor": z.string(),
  "environment": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "fork_parent": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "previous_revision_id": z.string().nullable().exactOptional(),
  "reason": z.string(),
  "replayable": z.boolean(),
  "request_id": z.string().nullable().exactOptional(),
  "timestamp_unix_ms": z.int(),
});
