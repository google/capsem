// Generated from Capsem OpenAPI. Do not edit.

import type { AuditEvent } from "./AuditEvent.js";
import type { CredentialEvent } from "./CredentialEvent.js";
import type { DnsEvent } from "./DnsEvent.js";
import type { EventBody } from "./EventBody.js";
import type { FileEvent } from "./FileEvent.js";
import type { HttpEvent } from "./HttpEvent.js";
import type { ModelEvent } from "./ModelEvent.js";
import type { ModelUsage } from "./ModelUsage.js";
import type { ProcessEvent } from "./ProcessEvent.js";
import type { ToolEvent } from "./ToolEvent.js";

export interface VmStatsDetailResponse {
  "audit_events": Array<AuditEvent>;
  "body_blobs": Record<string, Array<EventBody>>;
  "credential_events": Array<CredentialEvent>;
  "dns_events": Array<DnsEvent>;
  "file_events": Array<FileEvent>;
  "http_events": Array<HttpEvent>;
  "model_events": Array<ModelEvent>;
  "model_stats": Array<ModelUsage>;
  "process_events": Array<ProcessEvent>;
  "tool_events": Array<ToolEvent>;
}
