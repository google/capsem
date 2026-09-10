// Generated from Capsem OpenAPI. Do not edit.



export interface ProvisionRequest {
  "cpus"?: number | null;
  "env"?: Record<string, string> | null;
  "from"?: string | null;
  "name"?: string | null;
  "persistent"?: boolean;
  "profile_id": string;
  "ram_mb"?: number | null;
}
