// Generated from Capsem OpenAPI. Do not edit.

import type { AssetStatus } from "./AssetStatus.js";
import type { ResourceSummary } from "./ResourceSummary.js";
import type { ServiceAvailability } from "./ServiceAvailability.js";
import type { UpdateStatusResponse } from "./UpdateStatusResponse.js";
import type { VmSummary } from "./VmSummary.js";

export interface HypervisorInfo {
  "assets"?: null | AssetStatus;
  "gateway_version": string;
  "resource_summary"?: null | ResourceSummary;
  "service": ServiceAvailability;
  "updates"?: null | UpdateStatusResponse;
  "vm_count": number;
  "vms": Array<VmSummary>;
}
