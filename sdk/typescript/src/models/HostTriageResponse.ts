// Generated from Capsem OpenAPI. Do not edit.

import type { ErrorEvent } from "./ErrorEvent.js";
import type { PanicEvent } from "./PanicEvent.js";
import type { SlowOpEvent } from "./SlowOpEvent.js";

export interface HostTriageResponse {
  "errors": Array<ErrorEvent>;
  "panics": Array<PanicEvent>;
  "slow_ops": Array<SlowOpEvent>;
}
