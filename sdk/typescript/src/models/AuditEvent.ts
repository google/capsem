// Generated from Capsem OpenAPI. Do not edit.



export interface AuditEvent {
  "argv": string;
  "audit_id"?: string | null;
  "comm"?: string | null;
  "credential_ref"?: string | null;
  "cwd"?: string | null;
  "event_id": string;
  "exe": string;
  "exec_event_id"?: number | null;
  "exit_code"?: number | null;
  "parent_exe"?: string | null;
  "pid": number;
  "ppid": number;
  "session_id"?: number | null;
  "timestamp": string;
  "trace_id"?: string | null;
  "tty"?: string | null;
  "uid": number;
}
