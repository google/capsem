// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmSkillListResponse} from "../models/VmSkillListResponse.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";
import {EnvironmentSkillSummarySchema} from "./EnvironmentSkillSummary.js";

export const VmSkillListResponseSchema: z.ZodType<VmSkillListResponse> = z.strictObject({
  "discovery_fresh": z.boolean(),
  "next_cursor": z.string().nullable().exactOptional(),
  "owner_generation": z.uuid(),
  "revision": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "skills": z.array(z.lazy(() => EnvironmentSkillSummarySchema)),
  "vm_id": z.string(),
});
