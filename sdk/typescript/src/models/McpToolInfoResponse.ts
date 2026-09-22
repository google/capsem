// Generated from Capsem OpenAPI. Do not edit.

import type { JSONType } from "zod";
import type { McpPermissionAction } from "./McpPermissionAction.js";

export interface McpToolInfoResponse {
  "annotations"?: JSONType;
  "description"?: string | null;
  "namespaced_name": string;
  "original_name": string;
  "permission_action": McpPermissionAction;
  "permission_source": string;
  "pin_changed": boolean;
  "pin_hash"?: string | null;
  "server_name": string;
}
