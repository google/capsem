// Generated from Capsem OpenAPI. Do not edit.

import type { ProfileArtifactIssue } from "./ProfileArtifactIssue.js";
import type { ProfileUpdateSemantics } from "./ProfileUpdateSemantics.js";

export interface ProfileReadiness {
  "asset_count": number;
  "current_arch": string;
  "description": string;
  "errors": Array<string>;
  "id": string;
  "invalid_assets": Array<ProfileArtifactIssue>;
  "invalid_files": Array<ProfileArtifactIssue>;
  "missing_assets": Array<ProfileArtifactIssue>;
  "name": string;
  "profile_payload_hash"?: string | null;
  "ready": boolean;
  "revision"?: string | null;
  "update_semantics"?: null | ProfileUpdateSemantics;
}
