// Generated from Capsem OpenAPI. Do not edit.



export interface RunRequest {
  "command": string;
  "cpus"?: number | null;
  "env"?: Record<string, string> | null;
  "profile_id": string;
  "ram_mb"?: number | null;
  "timeout_secs"?: number | null;
}
