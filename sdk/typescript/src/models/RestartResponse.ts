// Generated from Capsem OpenAPI. Do not edit.

import type { RestartAuthentication } from "./RestartAuthentication.js";
import type { RestartStatus } from "./RestartStatus.js";
import type { ServiceManager } from "./ServiceManager.js";

export interface RestartResponse {
  "authentication": RestartAuthentication;
  "manager": ServiceManager;
  "status": RestartStatus;
}
