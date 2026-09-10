// Generated from Capsem OpenAPI. Do not edit.

import type { ProfileAvailabilitySummary } from "./ProfileAvailabilitySummary.js";
import type { ProfileUpdateSemantics } from "./ProfileUpdateSemantics.js";

export interface ProfileSummary {
  "availability": ProfileAvailabilitySummary;
  "default_rule_count": number;
  "description": string;
  "icon_svg"?: string | null;
  "id": string;
  "mcp_server_count": number;
  "name": string;
  "plugin_count": number;
  "rule_count": number;
  "source": string;
  "update_semantics": ProfileUpdateSemantics;
}
