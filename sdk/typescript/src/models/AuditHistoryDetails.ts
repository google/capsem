// Generated from Capsem OpenAPI. Do not edit.



export interface AuditHistoryDetails {
  "audit_id"?: string | null;
  "comm"?: string | null;
  "cwd"?: string | null;
  "exe": string;
  "parent_exe"?: string | null;
  "pid": number;
  "ppid": number;
  "session_id"?: number | null;
  "tty"?: string | null;
  "uid": number;
}
