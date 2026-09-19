// Generated from Capsem OpenAPI. Do not edit.

import type { FileChangeKind } from "./FileChangeKind.js";

export interface FileChange {
  "is_symlink": boolean;
  "kind": FileChangeKind;
  "path": string;
  "size"?: number | null;
}
