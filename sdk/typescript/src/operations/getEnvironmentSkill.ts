// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentSkillDetailResponse} from "../models/EnvironmentSkillDetailResponse.js";
import {EnvironmentSkillDetailResponseSchema} from "../validation/EnvironmentSkillDetailResponse.js";

export async function getEnvironmentSkill(
  transport: Transport,
  parameters: {
    "id": string;
    "skill_id": string;
    "revision_id"?: string | null;
  },
  options: CallOptions = {},
): Promise<EnvironmentSkillDetailResponse> {
  const input = z.object({
  "id": z.string(),
  "skill_id": z.string(),
  "revision_id": z.uuid().nullable().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/skills/{skill_id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "skill_id": input["skill_id"]},
    query: {"revision_id": input["revision_id"]},
  });
  return z.lazy(() => EnvironmentSkillDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
