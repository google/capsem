// Generated from Capsem OpenAPI. Do not edit.

import type { JSONType } from "zod";
import type { HostTriageResponse } from "./HostTriageResponse.js";

export interface TriageResponse {
  "host": HostTriageResponse;
  "rank": Array<string>;
  "session": JSONType;
  "session_id"?: string | null;
  "since": string;
}
