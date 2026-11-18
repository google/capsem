// Generated from Capsem OpenAPI. Do not edit.

import type { VmLifecycleSelection } from "./VmLifecycleSelection.js";

export interface ForkRequest {
  "description"?: string | null;
  "expected"?: null | VmLifecycleSelection;
  "labels"?: Record<string, string> | null;
  "name": string;
}
