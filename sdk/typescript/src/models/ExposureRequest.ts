// Generated from Capsem OpenAPI. Do not edit.

import type { ExposureAccess } from "./ExposureAccess.js";
import type { ExposureTarget } from "./ExposureTarget.js";

export interface ExposureRequest {
  "access"?: ExposureAccess;
  "guest_port": number;
  "host_port"?: number;
  "target"?: ExposureTarget;
}
