// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ProfilesListResponse} from "../models/ProfilesListResponse.js";
import {ProfilesListResponseSchema} from "../validation/ProfilesListResponse.js";

export async function listProfiles(
  transport: Transport,
  options: CallOptions = {},
): Promise<ProfilesListResponse> {
  const payload = await transport.request(Method.GET, "/profiles/list", {
    signal: options.signal, accept: MediaType.JSON,
  });
  return z.lazy(() => ProfilesListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
