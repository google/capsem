// Generated from Capsem OpenAPI. Do not edit.

import type { NetworkDecision } from "./NetworkDecision.js";
import type { NetworkProtocol } from "./NetworkProtocol.js";

export interface DnsEvent {
  "credential_ref"?: string | null;
  "decision": NetworkDecision;
  "event_id": string;
  "matched_rule"?: string | null;
  "policy_rule"?: string | null;
  "process_name"?: string | null;
  "qclass": number;
  "qname": string;
  "qtype": number;
  "rcode": number;
  "source_proto"?: null | NetworkProtocol;
  "timestamp": string;
  "trace_id"?: string | null;
  "upstream_resolver_ms"?: number | null;
}
