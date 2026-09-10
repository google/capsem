// Generated from Capsem OpenAPI. Do not edit.

import type { VmAction } from "./VmAction.js";
import type { VmLifecycleState } from "./VmLifecycleState.js";

export interface ProvisionResponse {
  "available_actions": Array<VmAction>;
  "can_resume"?: boolean;
  "id": string;
  "name": string;
  "persistent"?: boolean;
  "profile_id": string;
  "status": VmLifecycleState;
  "uds_path"?: string | null;
}
