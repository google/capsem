// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentListItem} from "../models/EnvironmentListItem.js";
import {EnvironmentListSummarySchema} from "./EnvironmentListSummary.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentListItemSchema: z.ZodType<EnvironmentListItem> = z.strictObject({
  "environment": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "instance_count": z.int().min(0).nullable().exactOptional(),
  "summary": z.union([z.null(), z.lazy(() => EnvironmentListSummarySchema)]).exactOptional(),
});
