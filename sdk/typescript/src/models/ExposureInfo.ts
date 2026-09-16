// Generated from Capsem OpenAPI. Do not edit.

import type { ExposureAccess } from "./ExposureAccess.js";
import type { ExposureTarget } from "./ExposureTarget.js";

export interface ExposureInfo {
  "access": ExposureAccess;
  "guest_port": number;
  "host_port"?: number | null;
  "id": string;
  "target": ExposureTarget;
}
