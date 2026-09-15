// Generated from Capsem OpenAPI. Do not edit.

import type { NetworkMemberInfo } from "./NetworkMemberInfo.js";

export interface NetworkInfo {
  "created_unix_ms": number;
  "id": string;
  "members": Array<NetworkMemberInfo>;
  "name": string;
  "subnet": string;
}
