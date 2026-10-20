// Generated from Capsem OpenAPI. Do not edit.

import type { ManagedSessionOwnership } from "./ManagedSessionOwnership.js";
import type { VmAction } from "./VmAction.js";
import type { VmLifecycleState } from "./VmLifecycleState.js";

export interface ProvisionResponse {
  "available_actions": Array<VmAction>;
  "can_resume"?: boolean;
  "id": string;
  "name": string;
  "ownership"?: null | ManagedSessionOwnership;
  "persistent"?: boolean;
  "status": VmLifecycleState;
}
