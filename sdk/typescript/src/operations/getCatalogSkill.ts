// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SkillCatalogDetailResponse} from "../models/SkillCatalogDetailResponse.js";
import {SkillCatalogDetailResponseSchema} from "../validation/SkillCatalogDetailResponse.js";

export async function getCatalogSkill(
  transport: Transport,
  parameters: {
    "skill_id": string;
    "version": string;
    "snapshot": string;
  },
  options: CallOptions = {},
): Promise<SkillCatalogDetailResponse> {
  const input = z.object({
  "skill_id": z.string(),
  "version": z.string(),
  "snapshot": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/skills/{skill_id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"skill_id": input["skill_id"]},
    query: {"version": input["version"], "snapshot": input["snapshot"]},
  });
  return z.lazy(() => SkillCatalogDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
