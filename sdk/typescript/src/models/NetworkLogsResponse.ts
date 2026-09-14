// Generated from Capsem OpenAPI. Do not edit.

import type { NetworkLogEvent } from "./NetworkLogEvent.js";

export interface NetworkLogsResponse {
  "cursor": string;
  "events": Array<NetworkLogEvent>;
  "next_cursor"?: string | null;
}
