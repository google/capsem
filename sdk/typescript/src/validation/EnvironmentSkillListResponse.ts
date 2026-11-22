// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSkillListResponse} from "../models/EnvironmentSkillListResponse.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";
import {EnvironmentSkillSummarySchema} from "./EnvironmentSkillSummary.js";

export const EnvironmentSkillListResponseSchema: z.ZodType<EnvironmentSkillListResponse> = z.strictObject({
  "discovery_fresh": z.boolean(),
  "next_cursor": z.string().nullable().exactOptional(),
  "revision": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "skills": z.array(z.lazy(() => EnvironmentSkillSummarySchema)),
});
