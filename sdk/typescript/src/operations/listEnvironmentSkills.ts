// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentSkillListResponse} from "../models/EnvironmentSkillListResponse.js";
import {EnvironmentSkillListResponseSchema} from "../validation/EnvironmentSkillListResponse.js";

export async function listEnvironmentSkills(
  transport: Transport,
  parameters: {
    "id": string;
    "revision_id"?: string;
    "search"?: string;
    "after"?: string;
    "limit"?: number;
  },
  options: CallOptions = {},
): Promise<EnvironmentSkillListResponse> {
  const input = z.object({
  "id": z.string(),
  "revision_id": z.uuid().exactOptional(),
  "search": z.string().exactOptional(),
  "after": z.string().exactOptional(),
  "limit": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/skills/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"revision_id": input["revision_id"], "search": input["search"], "after": input["after"], "limit": input["limit"]},
  });
  return z.lazy(() => EnvironmentSkillListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
