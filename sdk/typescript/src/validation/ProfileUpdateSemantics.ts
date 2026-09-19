// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileUpdateSemantics} from "../models/ProfileUpdateSemantics.js";
import {ProfileExistingVmUpdateSemanticsSchema} from "./ProfileExistingVmUpdateSemantics.js";
import {ProfileNewSessionUpdateSemanticsSchema} from "./ProfileNewSessionUpdateSemantics.js";
import {ProfileUpgradeActionSchema} from "./ProfileUpgradeAction.js";

export const ProfileUpdateSemanticsSchema: z.ZodType<ProfileUpdateSemantics> = z.object({
  "existing_vms": z.lazy(() => ProfileExistingVmUpdateSemanticsSchema),
  "new_sessions": z.lazy(() => ProfileNewSessionUpdateSemanticsSchema),
  "upgrade_action": z.lazy(() => ProfileUpgradeActionSchema),
});
