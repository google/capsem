// Generated from Capsem OpenAPI. Do not edit.

import type { FileEntryType } from "./FileEntryType.js";

export interface FileListEntry {
  "children"?: Array<FileListEntry> | null;
  "is_text"?: boolean | null;
  "label"?: string | null;
  "mime"?: string | null;
  "mtime": number;
  "name": string;
  "path": string;
  "size": number;
  "type": FileEntryType;
}
