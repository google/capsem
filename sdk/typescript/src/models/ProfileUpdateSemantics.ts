// Generated from Capsem OpenAPI. Do not edit.

import type { ProfileExistingVmUpdateSemantics } from "./ProfileExistingVmUpdateSemantics.js";
import type { ProfileNewSessionUpdateSemantics } from "./ProfileNewSessionUpdateSemantics.js";
import type { ProfileUpgradeAction } from "./ProfileUpgradeAction.js";

export interface ProfileUpdateSemantics {
  "existing_vms": ProfileExistingVmUpdateSemantics;
  "new_sessions": ProfileNewSessionUpdateSemantics;
  "upgrade_action": ProfileUpgradeAction;
}
