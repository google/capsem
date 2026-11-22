// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmSkillDetailResponse} from "../models/VmSkillDetailResponse.js";
import {EnvironmentSkillDetailResponseSchema} from "./EnvironmentSkillDetailResponse.js";

export const VmSkillDetailResponseSchema: z.ZodType<VmSkillDetailResponse> = z.strictObject({
  "owner_generation": z.uuid(),
  "skill": z.lazy(() => EnvironmentSkillDetailResponseSchema),
  "vm_id": z.string(),
});
