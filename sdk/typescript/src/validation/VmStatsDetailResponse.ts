// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmStatsDetailResponse} from "../models/VmStatsDetailResponse.js";
import {AuditEventSchema} from "./AuditEvent.js";
import {CredentialEventSchema} from "./CredentialEvent.js";
import {DnsEventSchema} from "./DnsEvent.js";
import {EventBodySchema} from "./EventBody.js";
import {FileEventSchema} from "./FileEvent.js";
import {HttpEventSchema} from "./HttpEvent.js";
import {InteractionReportSchema} from "./InteractionReport.js";
import {ModelEventSchema} from "./ModelEvent.js";
import {ModelUsageSchema} from "./ModelUsage.js";
import {ProcessEventSchema} from "./ProcessEvent.js";
import {ToolEventSchema} from "./ToolEvent.js";

export const VmStatsDetailResponseSchema: z.ZodType<VmStatsDetailResponse> = z.object({
  "audit_events": z.array(z.lazy(() => AuditEventSchema)),
  "body_blobs": z.record(z.string(), z.array(z.lazy(() => EventBodySchema))),
  "credential_events": z.array(z.lazy(() => CredentialEventSchema)),
  "dns_events": z.array(z.lazy(() => DnsEventSchema)),
  "file_events": z.array(z.lazy(() => FileEventSchema)),
  "http_events": z.array(z.lazy(() => HttpEventSchema)),
  "interactions": z.lazy(() => InteractionReportSchema),
  "model_events": z.array(z.lazy(() => ModelEventSchema)),
  "model_stats": z.array(z.lazy(() => ModelUsageSchema)),
  "process_events": z.array(z.lazy(() => ProcessEventSchema)),
  "tool_events": z.array(z.lazy(() => ToolEventSchema)),
});
