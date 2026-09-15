// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ProfileMcpInfoResponse} from "../models/ProfileMcpInfoResponse.js";
import {ProfileMcpInfoResponseSchema} from "../validation/ProfileMcpInfoResponse.js";

export async function getProfileMcpInfo(
  transport: Transport,
  parameters: {
    "profile_id": string;
  },
  options: CallOptions = {},
): Promise<ProfileMcpInfoResponse> {
  const input = z.object({
  "profile_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/profiles/{profile_id}/mcp/info", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"profile_id": input["profile_id"]},
  });
  return z.lazy(() => ProfileMcpInfoResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
