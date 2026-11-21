// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRuleListResponse} from "../models/EnvironmentRuleListResponse.js";
import {EnvironmentEffectiveAccessRuleSchema} from "./EnvironmentEffectiveAccessRule.js";
import {EnvironmentEffectiveDetectionRuleSchema} from "./EnvironmentEffectiveDetectionRule.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentRuleListResponseSchema: z.ZodType<EnvironmentRuleListResponse> = z.strictObject({
  "access": z.array(z.lazy(() => EnvironmentEffectiveAccessRuleSchema)),
  "detection": z.array(z.lazy(() => EnvironmentEffectiveDetectionRuleSchema)),
  "revision": z.lazy(() => EnvironmentRevisionReferenceSchema),
});
