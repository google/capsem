// Generated from Capsem OpenAPI. Do not edit.



export interface ModelEvent {
  "credential_ref"?: string | null;
  "duration_ms"?: number | null;
  "event_id": string;
  "input_tokens"?: number | null;
  "method": string;
  "model"?: string | null;
  "output_tokens"?: number | null;
  "path": string;
  "provider": string;
  "response_bytes"?: number | null;
  "status_code"?: number | null;
  "stop_reason"?: string | null;
  "timestamp": string;
  "trace_id"?: string | null;
}
