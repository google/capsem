// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ScopeProfileSummary} from "../models/ScopeProfileSummary.js";
import {CapabilitySummarySchema} from "./CapabilitySummary.js";
import {EnrollmentKindSchema} from "./EnrollmentKind.js";

export const ScopeProfileSummarySchema: z.ZodType<ScopeProfileSummary> = z.strictObject({
  "application_label": z.string(),
  "capabilities": z.array(z.lazy(() => CapabilitySummarySchema)),
  "enabled": z.boolean(),
  "enrollment_kind": z.lazy(() => EnrollmentKindSchema),
  "id": z.string(),
  "label": z.string(),
  "requested_scopes": z.array(z.string()),
});
