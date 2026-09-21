// Generated from Capsem OpenAPI. Do not edit.

import type { FileChange } from "./FileChange.js";

export interface ChangesResponse {
  "changes": Array<FileChange>;
  "checkpoint": string;
  "has_more": boolean;
  "total": number;
}
